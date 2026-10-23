//! The eval harness (see docs/evals.md): runs a workflow's labelled dataset through the
//! real job worker and runner in a throwaway database, scores each run against its labels
//! and reports the pass rate with the tokens, cost and duration of every case.
//!
//! The harness never approves or rejects a pending action: a run parked at the approval
//! gate is a final state, and what it proposed is scored. No `external` handler runs.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, Connection, PgPool};
use tokio::sync::watch;

use crate::claude::ClaudeRunner;
use crate::events::EventBus;
use crate::jobs::{self, Wake, Worker};
use crate::runs::{self, RunStatus, Runtime};

/// How often a case's run status is polled.
const POLL: Duration = Duration::from_millis(200);
/// The case timeout when none is given: long enough for a usage-limit pause.
pub const DEFAULT_CASE_TIMEOUT: Duration = Duration::from_secs(3600);
/// The prefix of every scratch database's name.
const SCRATCH_PREFIX: &str = "waypoint_eval_";

/// A workflow's eval suite: the workflow it runs and how a case's `setup` seeds rows.
#[rocket::async_trait]
pub trait Suite: Send + Sync {
    /// The registered workflow the suite's cases run.
    fn workflow(&self) -> &'static str;

    /// Seeds what a case needs from its `setup` value and returns the variables its
    /// `${name}` placeholders may use.
    async fn setup(&self, pool: &PgPool, setup: &Value) -> Result<Map<String, Value>, String>;
}

/// A dataset file: the workflow and its labelled cases, in run order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    pub workflow: String,
    pub cases: Vec<Case>,
}

/// One labelled case.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    /// Handed to [`Suite::setup`]; `null` when absent.
    #[serde(default)]
    pub setup: Value,
    /// The run's input, after placeholder substitution.
    pub input: Value,
    pub expect: Expect,
}

/// What a case's run must look like once it has settled.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub status: RunStatus,
    /// Subset-matched against the run's `output`.
    #[serde(default)]
    pub output: Option<Value>,
    /// A substring of the run's `error`.
    #[serde(default)]
    pub error_contains: Option<String>,
    /// `{tool, status, input}` per pending action, by id.
    #[serde(default)]
    pub pending_actions: Option<Vec<Value>>,
    /// `{kind, confidence, payload}` per review item, by id.
    #[serde(default)]
    pub review_items: Option<Vec<Value>>,
    /// `{tool, undone}` per logged action, by id.
    #[serde(default)]
    pub actions: Option<Vec<Value>>,
    /// `{step, memory_id}` per memory the run's steps retrieved (`run_memories`), by id.
    #[serde(default)]
    pub memories: Option<Vec<Value>>,
}

impl Expect {
    /// The labels with every `${name}` placeholder replaced (see [`substitute`]).
    pub fn substitute(&self, vars: &Map<String, Value>) -> Result<Self, String> {
        let list = |items: &Option<Vec<Value>>| -> Result<Option<Vec<Value>>, String> {
            items
                .as_ref()
                .map(|items| items.iter().map(|item| substitute(item, vars)).collect())
                .transpose()
        };
        Ok(Self {
            status: self.status,
            output: self
                .output
                .as_ref()
                .map(|output| substitute(output, vars))
                .transpose()?,
            error_contains: self.error_contains.clone(),
            pending_actions: list(&self.pending_actions)?,
            review_items: list(&self.review_items)?,
            actions: list(&self.actions)?,
            memories: list(&self.memories)?,
        })
    }
}

/// The statuses a run can end a case in.
fn is_settled(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Succeeded
            | RunStatus::Failed
            | RunStatus::AwaitingApproval
            | RunStatus::Rejected
    )
}

fn status_name(status: RunStatus) -> String {
    match serde_json::to_value(status) {
        Ok(Value::String(name)) => name,
        _ => format!("{status:?}"),
    }
}

/// `api/evals/<workflow>.json`, where a suite's dataset lives by default.
pub fn default_path(workflow: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evals")
        .join(format!("{workflow}.json"))
}

/// Reads and validates the dataset at `path` for `workflow`. Errors name the file.
pub fn load(path: &Path, workflow: &str) -> Result<Dataset, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    parse(&text, workflow).map_err(|err| format!("{}: {err}", path.display()))
}

/// The top level of a dataset, before its cases are read one by one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDataset {
    workflow: String,
    cases: Vec<Value>,
}

/// Parses and validates a dataset's JSON text for `workflow`. Errors name the case where
/// one is at fault.
pub fn parse(text: &str, workflow: &str) -> Result<Dataset, String> {
    let raw: RawDataset =
        serde_json::from_str(text).map_err(|err| format!("invalid dataset: {err}"))?;
    if raw.workflow != workflow {
        return Err(format!(
            "the dataset is for workflow {:?}, not {workflow:?}",
            raw.workflow
        ));
    }
    if raw.cases.is_empty() {
        return Err("the dataset has no cases".into());
    }
    let mut cases = Vec::with_capacity(raw.cases.len());
    let mut names = HashSet::new();
    for (index, value) in raw.cases.into_iter().enumerate() {
        let label = match value.get("name").and_then(Value::as_str) {
            Some(name) if !name.trim().is_empty() => format!("case {:?}", name),
            _ => format!("case {}", index + 1),
        };
        let case = Case::deserialize(&value).map_err(|err| format!("{label}: {err}"))?;
        if case.name.trim().is_empty() {
            return Err(format!("{label}: the name is empty"));
        }
        if !names.insert(case.name.clone()) {
            return Err(format!("{label}: another case has the same name"));
        }
        if !is_settled(case.expect.status) {
            return Err(format!(
                "{label}: expect.status must be succeeded, failed, awaiting_approval or \
                 rejected, got {}",
                status_name(case.expect.status)
            ));
        }
        cases.push(case);
    }
    Ok(Dataset {
        workflow: raw.workflow,
        cases,
    })
}

impl Dataset {
    /// The cases named in `names`, in dataset order; every case when `names` is empty. A
    /// name the dataset doesn't have is an error.
    pub fn select(&self, names: &[String]) -> Result<Vec<&Case>, String> {
        if let Some(unknown) = names
            .iter()
            .find(|name| !self.cases.iter().any(|case| &case.name == *name))
        {
            return Err(format!("the dataset has no case named {unknown:?}"));
        }
        Ok(self
            .cases
            .iter()
            .filter(|case| names.is_empty() || names.contains(&case.name))
            .collect())
    }
}

fn join_key(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn shown(path: &str) -> &str {
    if path.is_empty() { "$" } else { path }
}

/// Every way `actual` fails to contain `expected`, each prefixed by its JSON path. An
/// object matches when each expected key exists and matches; an array when it has the
/// same length and each element matches in order; numbers by `f64` value; other scalars
/// with `==`.
pub fn subset(expected: &Value, actual: &Value, path: &str) -> Vec<String> {
    let mut failures = Vec::new();
    subset_into(expected, actual, path, &mut failures);
    failures
}

fn subset_into(expected: &Value, actual: &Value, path: &str, failures: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            for (key, value) in expected {
                let at = join_key(path, key);
                match actual.get(key) {
                    Some(found) => subset_into(value, found, &at, failures),
                    None => failures.push(format!("{at}: missing, expected {value}")),
                }
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            if expected.len() != actual.len() {
                failures.push(format!(
                    "{}: expected {} item(s), got {}",
                    shown(path),
                    expected.len(),
                    actual.len()
                ));
                return;
            }
            for (index, (value, found)) in expected.iter().zip(actual).enumerate() {
                subset_into(value, found, &format!("{path}[{index}]"), failures);
            }
        }
        (Value::Number(want), Value::Number(got)) => {
            if want.as_f64() != got.as_f64() {
                failures.push(format!("{}: expected {want}, got {got}", shown(path)));
            }
        }
        _ => {
            if expected != actual {
                failures.push(format!(
                    "{}: expected {expected}, got {actual}",
                    shown(path)
                ));
            }
        }
    }
}

/// The variable a string that is exactly `${name}` names.
fn placeholder(text: &str) -> Option<&str> {
    let name = text.strip_prefix("${")?.strip_suffix('}')?;
    (!name.is_empty() && !name.contains(['{', '}', '$'])).then_some(name)
}

/// `value` with each string that is exactly `${name}` replaced by that variable's JSON
/// value, at any depth. Strings that only contain a placeholder are left alone. An
/// unknown variable is an error naming it.
pub fn substitute(value: &Value, vars: &Map<String, Value>) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => match placeholder(text) {
            Some(name) => vars
                .get(name)
                .cloned()
                .ok_or_else(|| format!("setup returned no variable named {name:?}"))?,
            None => value.clone(),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute(item, vars))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| Ok((key.clone(), substitute(item, vars)?)))
                .collect::<Result<_, String>>()?,
        ),
        _ => value.clone(),
    })
}

/// What a settled run did, as the labels see it. Each list is ordered by id.
#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub status: RunStatus,
    pub output: Option<Value>,
    pub error: Option<String>,
    /// `{tool, status, input}`.
    pub pending_actions: Vec<Value>,
    /// `{kind, confidence, payload}`.
    pub review_items: Vec<Value>,
    /// `{tool, undone}`.
    pub actions: Vec<Value>,
    /// `{step, memory_id}`.
    pub memories: Vec<Value>,
}

/// Every check of `expect` that `observed` fails; empty when the case passes.
pub fn score(expect: &Expect, observed: &Observed) -> Vec<String> {
    let mut failures = Vec::new();
    if expect.status != observed.status {
        failures.push(format!(
            "status: expected {}, got {}",
            status_name(expect.status),
            status_name(observed.status)
        ));
    }
    if let Some(output) = &expect.output {
        let actual = observed.output.clone().unwrap_or(Value::Null);
        failures.extend(subset(output, &actual, "output"));
    }
    if let Some(needle) = &expect.error_contains {
        match &observed.error {
            Some(error) if error.contains(needle.as_str()) => {}
            Some(error) => failures.push(format!(
                "error: expected it to contain {needle:?}, got {error:?}"
            )),
            None => failures.push(format!(
                "error: expected it to contain {needle:?}, got none"
            )),
        }
    }
    let lists = [
        (
            "pending_actions",
            &expect.pending_actions,
            &observed.pending_actions,
        ),
        ("review_items", &expect.review_items, &observed.review_items),
        ("actions", &expect.actions, &observed.actions),
        ("memories", &expect.memories, &observed.memories),
    ];
    for (name, expected, actual) in lists {
        if let Some(expected) = expected {
            failures.extend(subset(
                &Value::Array(expected.clone()),
                &Value::Array(actual.clone()),
                name,
            ));
        }
    }
    failures
}

/// How a case ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseResult {
    Pass,
    Fail,
    /// Setup, a placeholder, the database or the case timeout failed.
    Error,
    /// Not run, because an earlier case timed out.
    Skipped,
}

/// The model calls a case made, read from `claude_calls` (failed calls included).
#[derive(Debug, Clone, Default, PartialEq, Serialize, sqlx::FromRow)]
pub struct CallUsage {
    pub calls: i64,
    pub usage_limit_hits: i64,
    /// Input tokens including cache creation and cache reads.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    /// Sum of the calls' `duration_ms`.
    pub call_ms: i64,
}

impl CallUsage {
    fn add(&mut self, other: &Self) {
        self.calls += other.calls;
        self.usage_limit_hits += other.usage_limit_hits;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cost_usd += other.cost_usd;
        self.call_ms += other.call_ms;
    }
}

/// One case's outcome.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaseReport {
    pub name: String,
    pub result: CaseResult,
    pub run_id: Option<i64>,
    /// The run's last status; `null` when no run started.
    pub status: Option<RunStatus>,
    #[serde(flatten)]
    pub usage: CallUsage,
    /// From `runs::start` until the run settled (or the case timed out).
    pub wall_ms: i64,
    /// The checks that failed.
    pub failures: Vec<String>,
    pub error: Option<String>,
}

impl CaseReport {
    fn new(name: &str, result: CaseResult) -> Self {
        Self {
            name: name.to_owned(),
            result,
            run_id: None,
            status: None,
            usage: CallUsage::default(),
            wall_ms: 0,
            failures: Vec::new(),
            error: None,
        }
    }

    fn errored(mut self, error: String) -> Self {
        self.result = CaseResult::Error;
        self.error = Some(error);
        self
    }
}

/// Sums over every case, and means over the cases that ran.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Totals {
    pub cases: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub skipped: usize,
    /// `passed / cases`; skipped cases count against it.
    pub pass_rate: f64,
    #[serde(flatten)]
    pub usage: CallUsage,
    pub wall_ms: i64,
    pub input_tokens_per_run: f64,
    pub output_tokens_per_run: f64,
    pub cost_per_run_usd: f64,
}

impl Totals {
    pub fn of(cases: &[CaseReport]) -> Self {
        let count = |result| cases.iter().filter(|case| case.result == result).count();
        let mut usage = CallUsage::default();
        for case in cases {
            usage.add(&case.usage);
        }
        let skipped = count(CaseResult::Skipped);
        let passed = count(CaseResult::Pass);
        let ran = cases.len() - skipped;
        let per_run = |sum: f64| if ran == 0 { 0.0 } else { sum / ran as f64 };
        Self {
            cases: cases.len(),
            passed,
            failed: count(CaseResult::Fail),
            errors: count(CaseResult::Error),
            skipped,
            pass_rate: if cases.is_empty() {
                0.0
            } else {
                passed as f64 / cases.len() as f64
            },
            input_tokens_per_run: per_run(usage.input_tokens as f64),
            output_tokens_per_run: per_run(usage.output_tokens as f64),
            cost_per_run_usd: per_run(usage.cost_usd),
            wall_ms: cases.iter().map(|case| case.wall_ms).sum(),
            usage,
        }
    }
}

/// A finished eval.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub workflow: String,
    pub dataset: String,
    pub cases: Vec<CaseReport>,
    pub totals: Totals,
}

impl Report {
    pub fn new(workflow: &str, dataset: &str, cases: Vec<CaseReport>) -> Self {
        Self {
            workflow: workflow.to_owned(),
            dataset: dataset.to_owned(),
            totals: Totals::of(&cases),
            cases,
        }
    }

    /// Whether there were cases and every one passed.
    pub fn passed(&self) -> bool {
        self.totals.cases > 0 && self.totals.passed == self.totals.cases
    }

    /// The text report: a header, one row per case with its failed checks or error
    /// indented under it, and a final `pass rate:` line with the totals.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "eval {} · {} · {} case(s)",
            self.workflow, self.dataset, self.totals.cases
        );
        let width = self
            .cases
            .iter()
            .map(|case| case.name.chars().count())
            .max()
            .unwrap_or(0);
        for case in &self.cases {
            let result = match case.result {
                CaseResult::Pass => "pass",
                CaseResult::Fail => "FAIL",
                CaseResult::Error => "ERROR",
                CaseResult::Skipped => "skip",
            };
            let status = case.status.map_or_else(|| "-".to_owned(), status_name);
            let usage = &case.usage;
            let _ = writeln!(
                out,
                "{result:<5}  {:<width$}  {status:<17}  calls {}  in {}  out {}  ${:.4}  \
                 call {}ms  wall {}ms",
                case.name,
                usage.calls,
                usage.input_tokens,
                usage.output_tokens,
                usage.cost_usd,
                usage.call_ms,
                case.wall_ms
            );
            for failure in &case.failures {
                let _ = writeln!(out, "       - {failure}");
            }
            if let Some(error) = &case.error {
                let _ = writeln!(out, "       ! {error}");
            }
        }
        let totals = &self.totals;
        let pct = totals.pass_rate * 100.0;
        let _ = write!(
            out,
            "pass rate: {}/{} ({pct:.1}%) · {} failed · {} error(s) · {} skipped · calls {} \
             ({} usage limit) · in {} · out {} · ${:.4} · call {}ms · wall {}ms · per run: \
             in {:.1} · out {:.1} · ${:.4}",
            totals.passed,
            totals.cases,
            totals.failed,
            totals.errors,
            totals.skipped,
            totals.usage.calls,
            totals.usage.usage_limit_hits,
            totals.usage.input_tokens,
            totals.usage.output_tokens,
            totals.usage.cost_usd,
            totals.usage.call_ms,
            totals.wall_ms,
            totals.input_tokens_per_run,
            totals.output_tokens_per_run,
            totals.cost_per_run_usd
        );
        out
    }
}

/// How to run a dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Only these cases, in dataset order; every case when empty.
    pub cases: Vec<String>,
    /// How long one case's run may take to settle.
    pub case_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cases: Vec::new(),
            case_timeout: DEFAULT_CASE_TIMEOUT,
        }
    }
}

/// Runs `dataset`'s selected cases one at a time, in order, through a real [`Worker`] on
/// `pool`, and scores each. `Err` only when nothing could run (an unknown case name or a
/// workflow `runtime` lacks); a failure during a case makes that case an `error`. A case
/// that doesn't settle in time stops the eval and skips the rest, because its run would
/// still be spending calls.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    pool: &PgPool,
    runner: ClaudeRunner,
    runtime: Runtime,
    suite: &dyn Suite,
    jobs: jobs::Config,
    dataset: &Dataset,
    dataset_path: &str,
    options: &Options,
) -> Result<Report, String> {
    let selected = dataset.select(&options.cases)?;
    if runtime.workflow(suite.workflow()).is_none() {
        return Err(format!(
            "the runtime has no workflow named {:?}",
            suite.workflow()
        ));
    }
    let bus = EventBus::new();
    let wake = Wake::default();
    let (stop, stopped) = watch::channel(false);
    let worker = Worker {
        pool: pool.clone(),
        runner,
        bus: bus.clone(),
        config: jobs,
        wake: wake.0.clone(),
        runtime: runtime.clone(),
        // Not configured: the scratch database never syncs Gmail.
        google: crate::google::Google::new(crate::google::Config::default()),
    }
    .spawn(stopped);

    let mut reports = Vec::with_capacity(selected.len());
    let mut halted = false;
    for case in selected {
        if halted {
            reports.push(CaseReport::new(&case.name, CaseResult::Skipped));
            continue;
        }
        let (report, timed_out) = run_case(
            pool,
            &bus,
            &wake,
            &runtime,
            suite,
            case,
            options.case_timeout,
        )
        .await;
        halted = timed_out;
        reports.push(report);
    }

    stop.send_replace(true);
    if let Err(err) = worker.await {
        rocket::warn!("evals: the worker ended abnormally: {err}");
    }
    Ok(Report::new(suite.workflow(), dataset_path, reports))
}

/// Runs and scores one case. The flag is true when its run didn't settle in time.
async fn run_case(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    runtime: &Runtime,
    suite: &dyn Suite,
    case: &Case,
    timeout: Duration,
) -> (CaseReport, bool) {
    let report = CaseReport::new(&case.name, CaseResult::Pass);
    let vars = match suite.setup(pool, &case.setup).await {
        Ok(vars) => vars,
        Err(err) => return (report.errored(format!("setup failed: {err}")), false),
    };
    let input = match substitute(&case.input, &vars) {
        Ok(input) => input,
        Err(err) => return (report.errored(format!("input: {err}")), false),
    };
    let expect = match case.expect.substitute(&vars) {
        Ok(expect) => expect,
        Err(err) => return (report.errored(format!("expect: {err}")), false),
    };
    let watermark: i64 = match sqlx::query_scalar("SELECT COALESCE(max(id), 0) FROM claude_calls")
        .fetch_one(pool)
        .await
    {
        Ok(watermark) => watermark,
        Err(err) => {
            return (
                report.errored(format!("cannot read claude_calls: {err}")),
                false,
            );
        }
    };

    let started = Instant::now();
    let run = match runs::start(pool, bus, wake, runtime, suite.workflow(), input).await {
        Ok(run) => run,
        Err(err) => {
            return (
                report.errored(format!("cannot start the run: {err}")),
                false,
            );
        }
    };
    let mut report = CaseReport {
        run_id: Some(run.id),
        status: Some(run.status),
        ..report
    };
    let deadline = started + timeout;
    let mut timed_out = false;
    loop {
        let status: RunStatus =
            match sqlx::query_scalar("SELECT status FROM agent_runs WHERE id = $1")
                .bind(run.id)
                .fetch_one(pool)
                .await
            {
                Ok(status) => status,
                Err(err) => {
                    return (
                        report.errored(format!("cannot read run {}: {err}", run.id)),
                        false,
                    );
                }
            };
        report.status = Some(status);
        if is_settled(status) {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            timed_out = true;
            break;
        }
        tokio::time::sleep(POLL.min(deadline - now)).await;
    }
    report.wall_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);

    match usage(pool, watermark).await {
        Ok(usage) => report.usage = usage,
        Err(err) => {
            return (
                report.errored(format!("cannot read claude_calls: {err}")),
                timed_out,
            );
        }
    }
    if timed_out {
        let message = format!("did not settle within {}s", timeout.as_secs());
        return (report.errored(message), true);
    }
    match observe(pool, run.id).await {
        Ok(observed) => {
            report.failures = score(&expect, &observed);
            if !report.failures.is_empty() {
                report.result = CaseResult::Fail;
            }
            (report, false)
        }
        Err(err) => (
            report.errored(format!("cannot read run {}: {err}", run.id)),
            false,
        ),
    }
}

/// The model calls logged after `watermark`.
async fn usage(pool: &PgPool, watermark: i64) -> Result<CallUsage, sqlx::Error> {
    sqlx::query_as(
        "SELECT count(*) AS calls, \
         count(*) FILTER (WHERE status = 'usage_limit') AS usage_limit_hits, \
         COALESCE(sum(COALESCE(input_tokens, 0) + COALESCE(cache_creation_input_tokens, 0) \
         + COALESCE(cache_read_input_tokens, 0)), 0)::bigint AS input_tokens, \
         COALESCE(sum(output_tokens), 0)::bigint AS output_tokens, \
         COALESCE(sum(cost_usd), 0)::float8 AS cost_usd, \
         COALESCE(sum(duration_ms), 0)::bigint AS call_ms \
         FROM claude_calls WHERE id > $1",
    )
    .bind(watermark)
    .fetch_one(pool)
    .await
}

/// What run `run_id` did, for scoring.
async fn observe(pool: &PgPool, run_id: i64) -> Result<Observed, sqlx::Error> {
    let (status, output, error): (RunStatus, Option<Value>, Option<String>) =
        sqlx::query_as("SELECT status, output, error FROM agent_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(pool)
            .await?;
    let rows = |sql: &'static str| async move {
        sqlx::query_scalar::<_, Value>(sql)
            .bind(run_id)
            .fetch_all(pool)
            .await
    };
    Ok(Observed {
        status,
        output,
        error,
        pending_actions: rows(
            "SELECT jsonb_build_object('tool', tool, 'status', status, 'input', input) \
             FROM pending_actions WHERE run_id = $1 ORDER BY id",
        )
        .await?,
        review_items: rows(
            "SELECT jsonb_build_object('kind', kind, 'confidence', confidence, \
             'payload', payload) FROM review_items WHERE run_id = $1 ORDER BY id",
        )
        .await?,
        actions: rows(
            "SELECT jsonb_build_object('tool', tool, 'undone', undone_at IS NOT NULL) \
             FROM agent_actions WHERE run_id = $1 ORDER BY id",
        )
        .await?,
        memories: rows(
            "SELECT jsonb_build_object('step', s.name, 'memory_id', rm.memory_id) \
             FROM run_memories rm JOIN run_steps s ON s.id = rm.run_step_id \
             WHERE rm.run_id = $1 ORDER BY rm.id",
        )
        .await?,
    })
}

/// Whether `name` has the shape `waypoint_eval_<digits>_<digits>`, the only names the
/// harness quotes into SQL.
pub fn is_scratch_name(name: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    name.strip_prefix(SCRATCH_PREFIX)
        .and_then(|rest| rest.split_once('_'))
        .is_some_and(|(ms, pid)| digits(ms) && digits(pid))
}

/// A throwaway, migrated database on the `DATABASE_URL` server, so an eval never writes
/// to the user's own. [`ScratchDb::drop`] removes it.
pub struct ScratchDb {
    admin: PgConnectOptions,
    name: String,
    pool: PgPool,
}

impl ScratchDb {
    /// Creates `waypoint_eval_<unix_ms>_<pid>` through the URL's database, connects to it
    /// and runs the migrations. A failure after the create drops it again.
    pub async fn create(database_url: &str) -> Result<Self, String> {
        let admin = PgConnectOptions::from_str(database_url)
            .map_err(|err| format!("DATABASE_URL is not a valid Postgres URL: {err}"))?;
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let name = format!("{SCRATCH_PREFIX}{unix_ms}_{}", std::process::id());
        if !is_scratch_name(&name) {
            return Err(format!("refusing an unexpected database name {name:?}"));
        }
        let mut conn = admin
            .connect()
            .await
            .map_err(|err| format!("cannot connect to the database at DATABASE_URL: {err}"))?;
        let created = sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&mut conn)
            .await;
        let _ = conn.close().await;
        created.map_err(|err| format!("cannot create the scratch database {name}: {err}"))?;

        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(admin.clone().database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(err) => {
                let _ = drop_database(&admin, &name).await;
                return Err(format!(
                    "cannot connect to the scratch database {name}: {err}"
                ));
            }
        };
        let scratch = Self { admin, name, pool };
        if let Err(err) = crate::MIGRATOR.run(&scratch.pool).await {
            let message = format!(
                "cannot migrate the scratch database {}: {err}",
                scratch.name
            );
            let _ = scratch.drop().await;
            return Err(message);
        }
        Ok(scratch)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Closes the pool and drops the database, forcing out any other connection.
    pub async fn drop(self) -> Result<(), String> {
        self.pool.close().await;
        drop_database(&self.admin, &self.name).await
    }
}

async fn drop_database(admin: &PgConnectOptions, name: &str) -> Result<(), String> {
    if !is_scratch_name(name) {
        return Err(format!("refusing to drop {name:?}"));
    }
    let failed = |err: sqlx::Error| format!("cannot drop the scratch database {name}: {err}");
    let mut conn = admin.connect().await.map_err(failed)?;
    let dropped = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
        .execute(&mut conn)
        .await;
    let _ = conn.close().await;
    dropped.map(|_| ()).map_err(failed)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn subset_match_cases() {
        let actual = json!({
            "status": "pending",
            "input": {"application_id": 7, "message": "Hi", "extra": true},
            "items": [1, 2, 3],
            "score": 1.0
        });
        let cases: [(&str, Value, &[&str]); 8] = [
            ("equal", actual.clone(), &[]),
            ("superset actual", json!({"input": {"message": "Hi"}}), &[]),
            ("1 equals 1.0", json!({"score": 1}), &[]),
            (
                "missing key",
                json!({"input": {"note": "x"}}),
                &["input.note: missing, expected \"x\""],
            ),
            (
                "wrong scalar",
                json!({"input": {"message": "Thanks"}}),
                &["input.message: expected \"Thanks\", got \"Hi\""],
            ),
            (
                "wrong length",
                json!({"items": [1, 2]}),
                &["items: expected 2 item(s), got 3"],
            ),
            (
                "out of order",
                json!({"items": [1, 3, 2]}),
                &["items[1]: expected 3, got 2", "items[2]: expected 2, got 3"],
            ),
            (
                "number mismatch",
                json!({"input": {"application_id": 7.5}}),
                &["input.application_id: expected 7.5, got 7"],
            ),
        ];
        for (label, expected, failures) in cases {
            assert_eq!(subset(&expected, &actual, ""), failures, "{label}");
        }
        assert_eq!(
            subset(&json!([1]), &json!("x"), ""),
            ["$: expected [1], got \"x\""]
        );
        assert_eq!(
            subset(&json!({"a": 1}), &json!({"a": 2}), "output"),
            ["output.a: expected 1, got 2"]
        );
    }

    #[test]
    fn placeholders_substitute_typed_values() {
        let mut vars = Map::new();
        vars.insert("application_id".into(), json!(42));
        vars.insert("tags".into(), json!(["a", "b"]));
        let input = json!({
            "application_id": "${application_id}",
            "nested": [{"id": "${application_id}", "tags": "${tags}"}],
            "inside": "id ${application_id} here",
            "prefix": "${application_id}x",
            "plain": 3
        });
        assert_eq!(
            substitute(&input, &vars).unwrap(),
            json!({
                "application_id": 42,
                "nested": [{"id": 42, "tags": ["a", "b"]}],
                "inside": "id ${application_id} here",
                "prefix": "${application_id}x",
                "plain": 3
            })
        );
        let err = substitute(&json!({"a": ["${missing}"]}), &vars).unwrap_err();
        assert!(err.contains("\"missing\""), "{err}");
    }

    fn dataset_with(cases: Value) -> String {
        json!({"workflow": "tidy_application", "cases": cases}).to_string()
    }

    fn case(name: &str, status: &str) -> Value {
        json!({"name": name, "input": {}, "expect": {"status": status}})
    }

    #[test]
    fn dataset_validation_cases() {
        let valid = dataset_with(json!([
            {"name": "a", "setup": {"x": 1}, "input": {"k": 1}, "expect": {
                "status": "awaiting_approval", "output": {}, "error_contains": "x",
                "pending_actions": [], "review_items": [], "actions": [], "memories": []
            }},
            case("b", "failed")
        ]));
        let dataset = parse(&valid, "tidy_application").unwrap();
        assert_eq!(dataset.cases.len(), 2);
        assert_eq!(dataset.cases[1].setup, Value::Null);
        assert_eq!(dataset.select(&["b".into()]).unwrap()[0].name, "b");
        assert!(dataset.select(&["c".into()]).unwrap_err().contains("\"c\""));

        let mut extra = case("extra", "failed");
        extra["expect"]["surprise"] = json!(1);
        let mut memories = case("memories", "failed");
        memories["expect"]["memories"] = json!({"step": "draft"});
        let cases = [
            (
                dataset_with(json!([memories])),
                "case \"memories\": invalid type: map, expected a sequence",
            ),
            (
                json!({"workflow": "tidy_application", "cases": [], "x": 1}).to_string(),
                "unknown field `x`",
            ),
            (
                dataset_with(json!([extra])),
                "case \"extra\": unknown field `surprise`",
            ),
            (dataset_with(json!([])), "no cases"),
            (
                dataset_with(json!([case(" ", "failed")])),
                "case 1: the name is empty",
            ),
            (
                dataset_with(json!([case("a", "failed"), case("a", "succeeded")])),
                "case \"a\": another case has the same name",
            ),
            (
                dataset_with(json!([case("q", "queued")])),
                "case \"q\": expect.status must be",
            ),
            (
                dataset_with(json!([case("u", "nope")])),
                "case \"u\": unknown variant",
            ),
            (
                json!({"workflow": "other", "cases": [case("a", "failed")]}).to_string(),
                "workflow \"other\", not \"tidy_application\"",
            ),
            ("{".into(), "invalid dataset"),
        ];
        for (text, expected) in cases {
            let err = parse(&text, "tidy_application").unwrap_err();
            assert!(err.contains(expected), "{expected:?} not in {err:?}");
        }

        let err = load(Path::new("/nonexistent/x.json"), "x").unwrap_err();
        assert!(err.contains("/nonexistent/x.json"), "{err}");
    }

    #[test]
    fn score_checks_memories() {
        let observed = Observed {
            status: RunStatus::AwaitingApproval,
            output: None,
            error: None,
            pending_actions: Vec::new(),
            review_items: Vec::new(),
            actions: Vec::new(),
            memories: vec![
                json!({"step": "draft", "memory_id": 4}),
                json!({"step": "draft", "memory_id": 3}),
            ],
        };
        let expect = |memories: Option<Value>| Expect {
            status: RunStatus::AwaitingApproval,
            output: None,
            error_contains: None,
            pending_actions: None,
            review_items: None,
            actions: None,
            memories: memories.map(|m| m.as_array().cloned().unwrap_or_default()),
        };
        let cases: [(&str, Option<Value>, &[&str]); 5] = [
            ("unchecked", None, &[]),
            (
                "same rows in order",
                Some(json!([
                    {"step": "draft", "memory_id": 4},
                    {"step": "draft", "memory_id": 3}
                ])),
                &[],
            ),
            (
                "wrong order",
                Some(json!([
                    {"step": "draft", "memory_id": 3},
                    {"step": "draft", "memory_id": 4}
                ])),
                &[
                    "memories[0].memory_id: expected 3, got 4",
                    "memories[1].memory_id: expected 4, got 3",
                ],
            ),
            (
                "wrong count",
                Some(json!([{"step": "draft", "memory_id": 4}])),
                &["memories: expected 1 item(s), got 2"],
            ),
            (
                "wrong step",
                Some(json!([
                    {"step": "note", "memory_id": 4},
                    {"step": "draft", "memory_id": 3}
                ])),
                &["memories[0].step: expected \"note\", got \"draft\""],
            ),
        ];
        for (label, memories, failures) in cases {
            assert_eq!(score(&expect(memories), &observed), failures, "{label}");
        }

        let mut vars = Map::new();
        vars.insert("memory_remote".into(), json!(4));
        let substituted = expect(Some(
            json!([{"step": "draft", "memory_id": "${memory_remote}"}]),
        ))
        .substitute(&vars)
        .unwrap();
        assert_eq!(
            substituted.memories,
            Some(vec![json!({"step": "draft", "memory_id": 4})])
        );
    }

    fn report_case(name: &str, result: CaseResult, calls: i64, cost: f64) -> CaseReport {
        CaseReport {
            usage: CallUsage {
                calls,
                usage_limit_hits: 0,
                input_tokens: calls * 100,
                output_tokens: calls * 10,
                cost_usd: cost,
                call_ms: calls * 5,
            },
            wall_ms: 7,
            status: Some(RunStatus::Succeeded),
            ..CaseReport::new(name, result)
        }
    }

    #[test]
    fn report_totals_and_text() {
        let mut failed = report_case("labels differ", CaseResult::Fail, 1, 0.5);
        failed.failures = vec!["output.text: expected \"a\", got \"b\"".into()];
        let errored = report_case("times out", CaseResult::Error, 0, 0.0)
            .errored("did not settle within 1s".into());
        let cases = vec![
            report_case("passes", CaseResult::Pass, 2, 1.0),
            failed,
            errored,
            CaseReport::new("skipped", CaseResult::Skipped),
        ];
        let report = Report::new("w", "evals/w.json", cases);
        let totals = &report.totals;
        assert_eq!(
            (
                totals.cases,
                totals.passed,
                totals.failed,
                totals.errors,
                totals.skipped
            ),
            (4, 1, 1, 1, 1)
        );
        assert_eq!(totals.pass_rate, 0.25);
        assert_eq!(totals.usage.calls, 3);
        assert_eq!(totals.usage.input_tokens, 300);
        assert_eq!(totals.usage.output_tokens, 30);
        assert_eq!(totals.usage.call_ms, 15);
        assert_eq!(totals.usage.cost_usd, 1.5);
        assert_eq!(totals.wall_ms, 21);
        assert_eq!(totals.input_tokens_per_run, 100.0);
        assert_eq!(totals.output_tokens_per_run, 10.0);
        assert_eq!(totals.cost_per_run_usd, 0.5);
        assert!(!report.passed());

        let text = report.text();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains("w · evals/w.json · 4 case(s)"), "{text}");
        let fail_row = lines.iter().position(|line| line.contains("labels differ"));
        let fail_row = fail_row.expect("a row for the failed case");
        assert!(lines[fail_row].starts_with("FAIL"), "{text}");
        assert!(lines[fail_row].contains("$0.5000"), "{text}");
        assert!(
            lines[fail_row + 1].contains("- output.text: expected \"a\", got \"b\""),
            "{text}"
        );
        assert!(text.contains("! did not settle within 1s"), "{text}");
        assert!(
            lines.last().unwrap().starts_with("pass rate: 1/4 (25.0%)"),
            "{text}"
        );

        let none = Totals::of(&[CaseReport::new("s", CaseResult::Skipped)]);
        assert_eq!(none.pass_rate, 0.0);
        assert_eq!(none.input_tokens_per_run, 0.0);
        assert_eq!(none.cost_per_run_usd, 0.0);

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["cases"][0]["result"], "pass");
        assert_eq!(json["cases"][0]["calls"], 2);
        assert_eq!(json["cases"][3]["error"], Value::Null);
        assert_eq!(json["totals"]["pass_rate"], 0.25);
    }

    #[test]
    fn scratch_names_have_one_shape() {
        for (name, ok) in [
            ("waypoint_eval_1761170000000_4242", true),
            ("waypoint_eval_1_2", true),
            ("waypoint_eval__2", false),
            ("waypoint_eval_1_", false),
            ("waypoint_eval_1_2_3", false),
            ("waypoint_eval_1_2\"; DROP", false),
            ("waypoint", false),
        ] {
            assert_eq!(is_scratch_name(name), ok, "{name}");
        }
    }
}
