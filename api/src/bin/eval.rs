//! `cargo run --bin eval -- <workflow> [--case <name>]... [--json] [--keep-db]
//! [--dataset <path>] [--case-timeout-secs <n>]`: runs a workflow's labelled eval set
//! through the real job worker and `claude -p` runner in a throwaway database on the
//! `DATABASE_URL` server, and prints the pass rate with each case's tokens, cost and
//! duration (see docs/evals.md). It never approves a pending action.
//!
//! Exit codes: 0 when every selected case passed, 1 when any failed, errored or was
//! skipped, 2 for a usage or setup error.
//!
//! Workflows whose model calls use tools need the `waypoint-mcp` relay, which
//! `cargo build -p waypoint-api --bins` builds beside this binary (where
//! `WAYPOINT_MCP_BIN` defaults to).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use waypoint_api::claude::ClaudeRunner;
use waypoint_api::evals::{self, Options, ScratchDb};
use waypoint_api::{jobs, workflows};

/// Parsed command-line arguments.
#[derive(Debug, Clone, PartialEq)]
struct Args {
    workflow: String,
    cases: Vec<String>,
    json: bool,
    keep_db: bool,
    dataset: Option<PathBuf>,
    case_timeout: Duration,
}

/// Why the arguments can't run: `--help` or nothing given, or a mistake. Both print the
/// usage and exit 2.
#[derive(Debug, Clone, PartialEq)]
enum Usage {
    Help,
    Error(String),
}

fn suite_names() -> Vec<&'static str> {
    workflows::eval_suites()
        .iter()
        .map(|suite| suite.workflow())
        .collect()
}

fn usage_text() -> String {
    format!(
        "usage: cargo run --bin eval -- <workflow> [--case <name>]... [--json] [--keep-db] \
         [--dataset <path>] [--case-timeout-secs <n>]\nsuites: {}",
        suite_names().join(", ")
    )
}

/// The value after `flag`; another flag doesn't count as one.
fn value(flag: &str, iter: &mut std::slice::Iter<'_, String>) -> Result<String, Usage> {
    match iter.next() {
        Some(value) if !value.starts_with("--") => Ok(value.clone()),
        _ => Err(Usage::Error(format!("{flag} needs a value"))),
    }
}

fn parse_args(args: &[String]) -> Result<Args, Usage> {
    let mut workflow = None;
    let mut cases = Vec::new();
    let mut json = false;
    let mut keep_db = false;
    let mut dataset = None;
    let mut case_timeout = evals::DEFAULT_CASE_TIMEOUT;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(Usage::Help),
            "--json" => json = true,
            "--keep-db" => keep_db = true,
            "--case" => cases.push(value("--case", &mut iter)?),
            "--dataset" => dataset = Some(PathBuf::from(value("--dataset", &mut iter)?)),
            "--case-timeout-secs" => {
                let raw = value("--case-timeout-secs", &mut iter)?;
                match raw.parse::<u64>() {
                    Ok(secs) if secs > 0 => case_timeout = Duration::from_secs(secs),
                    _ => {
                        return Err(Usage::Error(format!(
                            "--case-timeout-secs must be a positive number of seconds, got {raw:?}"
                        )));
                    }
                }
            }
            flag if flag.starts_with('-') => {
                return Err(Usage::Error(format!("unknown flag {flag:?}")));
            }
            name if workflow.is_none() => workflow = Some(name.to_owned()),
            extra => return Err(Usage::Error(format!("unexpected argument {extra:?}"))),
        }
    }
    let Some(workflow) = workflow else {
        return Err(if args.is_empty() {
            Usage::Help
        } else {
            Usage::Error("no workflow given".into())
        });
    };
    if !suite_names().contains(&workflow.as_str()) {
        return Err(Usage::Error(format!("no eval suite is named {workflow:?}")));
    }
    Ok(Args {
        workflow,
        cases,
        json,
        keep_db,
        dataset,
        case_timeout,
    })
}

#[rocket::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(args) => ExitCode::from(run(args).await),
        Err(usage) => {
            if let Usage::Error(message) = usage {
                eprintln!("eval: {message}");
            }
            eprintln!("{}", usage_text());
            ExitCode::from(2)
        }
    }
}

/// Runs the eval and returns the exit code.
async fn run(args: Args) -> u8 {
    match prepare(&args) {
        Ok((dataset, path, runner, jobs, database_url)) => {
            execute(&args, &dataset, &path, runner, jobs, &database_url).await
        }
        Err(message) => {
            eprintln!("eval: {message}");
            2
        }
    }
}

type Prepared = (evals::Dataset, String, ClaudeRunner, jobs::Config, String);

/// Everything that can be checked before a database is created.
fn prepare(args: &Args) -> Result<Prepared, String> {
    dotenvy::dotenv().ok();
    let path = args
        .dataset
        .clone()
        .unwrap_or_else(|| evals::default_path(&args.workflow));
    let dataset = evals::load(&path, &args.workflow)?;
    dataset.select(&args.cases)?;
    let runner = ClaudeRunner::from_env()?;
    let jobs = jobs::Config::from_env()?;
    let database_url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .ok_or("DATABASE_URL is not set (copy .env.example to .env)")?;
    Ok((
        dataset,
        path.display().to_string(),
        runner,
        jobs,
        database_url,
    ))
}

async fn execute(
    args: &Args,
    dataset: &evals::Dataset,
    path: &str,
    runner: ClaudeRunner,
    jobs: jobs::Config,
    database_url: &str,
) -> u8 {
    let Some(suite) = workflows::eval_suites()
        .into_iter()
        .find(|suite| suite.workflow() == args.workflow)
    else {
        eprintln!("eval: no eval suite is named {:?}", args.workflow);
        return 2;
    };
    let scratch = match ScratchDb::create(database_url).await {
        Ok(scratch) => scratch,
        Err(message) => {
            eprintln!("eval: {message}");
            return 2;
        }
    };
    let options = Options {
        cases: args.cases.clone(),
        case_timeout: args.case_timeout,
    };
    let result = evals::run(
        scratch.pool(),
        runner,
        workflows::runtime(),
        suite.as_ref(),
        jobs,
        dataset,
        path,
        &options,
    )
    .await;
    let code = match result {
        Ok(report) => {
            if args.json {
                match serde_json::to_string_pretty(&report) {
                    Ok(json) => println!("{json}"),
                    Err(err) => eprintln!("eval: cannot serialize the report: {err}"),
                }
            } else {
                println!("{}", report.text());
            }
            if report.passed() { 0 } else { 1 }
        }
        Err(message) => {
            eprintln!("eval: {message}");
            2
        }
    };
    if args.keep_db {
        eprintln!("eval: kept the scratch database {}", scratch.name());
    } else {
        let name = scratch.name().to_owned();
        if let Err(message) = scratch.drop().await {
            eprintln!("eval: {message}; drop it with DROP DATABASE \"{name}\" WITH (FORCE)");
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, Usage> {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        parse_args(&args)
    }

    fn args(workflow: &str) -> Args {
        Args {
            workflow: workflow.into(),
            cases: Vec::new(),
            json: false,
            keep_db: false,
            dataset: None,
            case_timeout: Duration::from_secs(3600),
        }
    }

    #[test]
    fn parse_args_cases() {
        let all = Args {
            cases: vec!["a".into(), "b c".into()],
            json: true,
            keep_db: true,
            dataset: Some(PathBuf::from("x.json")),
            case_timeout: Duration::from_secs(5),
            ..args("tidy_application")
        };
        let cases: Vec<(&[&str], Result<Args, Usage>)> = vec![
            (&["tidy_application"], Ok(args("tidy_application"))),
            (
                &[
                    "--json",
                    "tidy_application",
                    "--case",
                    "a",
                    "--keep-db",
                    "--case",
                    "b c",
                    "--dataset",
                    "x.json",
                    "--case-timeout-secs",
                    "5",
                ],
                Ok(all),
            ),
            (&[], Err(Usage::Help)),
            (&["--help"], Err(Usage::Help)),
            (&["tidy_application", "-h"], Err(Usage::Help)),
            (
                &["tidy_application", "--verbose"],
                Err(Usage::Error("unknown flag \"--verbose\"".into())),
            ),
            (&["--json"], Err(Usage::Error("no workflow given".into()))),
            (
                &["tidy_application", "--case"],
                Err(Usage::Error("--case needs a value".into())),
            ),
            (
                &["tidy_application", "--dataset", "--json"],
                Err(Usage::Error("--dataset needs a value".into())),
            ),
            (
                &["tidy_application", "--case-timeout-secs", "0"],
                Err(Usage::Error(
                    "--case-timeout-secs must be a positive number of seconds, got \"0\"".into(),
                )),
            ),
            (
                &["tidy_application", "other"],
                Err(Usage::Error("unexpected argument \"other\"".into())),
            ),
            (
                &["nope"],
                Err(Usage::Error("no eval suite is named \"nope\"".into())),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(parse(input), expected, "{input:?}");
        }
        assert!(usage_text().contains("suites: tidy_application"));
    }
}
