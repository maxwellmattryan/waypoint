//! The eval harness end to end: the shipped `tidy_application` dataset, per-case call
//! attribution, usage-limit back-off, failures, timeouts and skips, and the scratch
//! database. Model calls go to `fake-claude.sh`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use sqlx::PgPool;
use waypoint_api::claude::ClaudeRequest;
use waypoint_api::evals::{self, CaseResult, Options, Report, ScratchDb, Suite};
use waypoint_api::runs::{RunStatus, Runtime, StepContext, StepError, Workflow};
use waypoint_api::tools::Registry;
use waypoint_api::workflows::{self, classify_email, memory_writer, tidy};

/// The runner's own timeout; longer than any case timeout used here.
const RUNNER_TIMEOUT: Duration = Duration::from_secs(30);

/// One step, `ask`, that makes one model call and returns `{"text": <reply>}`. With
/// `{"call": false}` as input it returns `{"text": null}` without calling.
struct AskOnce;

#[rocket::async_trait]
impl Workflow for AskOnce {
    fn name(&self) -> &'static str {
        "ask_once"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["ask"]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        _step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        if input.get("call") == Some(&json!(false)) {
            return Ok(json!({"text": null}));
        }
        let response = ctx
            .call(&ClaudeRequest {
                prompt: "ping".into(),
                purpose: "test.eval".into(),
                model: None,
            })
            .await?;
        Ok(json!({"text": response.text}))
    }
}

/// The suite for `ask_once`: setup seeds nothing.
struct AskOnceSuite;

#[rocket::async_trait]
impl Suite for AskOnceSuite {
    fn workflow(&self) -> &'static str {
        "ask_once"
    }

    async fn setup(&self, _pool: &PgPool, _setup: &Value) -> Result<Map<String, Value>, String> {
        Ok(Map::new())
    }
}

fn ask_runtime() -> Runtime {
    Runtime::new(vec![Arc::new(AskOnce)], Registry::default()).expect("a valid runtime")
}

fn ask_dataset(cases: Value) -> evals::Dataset {
    let text = json!({"workflow": "ask_once", "cases": cases}).to_string();
    evals::parse(&text, "ask_once").expect("a valid dataset")
}

fn options(case_timeout: Duration) -> Options {
    Options {
        cases: Vec::new(),
        case_timeout,
    }
}

async fn run_ask(
    pool: &PgPool,
    mode: &str,
    config: waypoint_api::jobs::Config,
    cases: Value,
    case_timeout: Duration,
) -> Report {
    let fake = common::fake(mode, RUNNER_TIMEOUT);
    evals::run(
        pool,
        fake.runner.clone(),
        ask_runtime(),
        &AskOnceSuite,
        config,
        &ask_dataset(cases),
        "ask_once.json",
        &options(case_timeout),
    )
    .await
    .expect("the eval runs")
}

#[sqlx::test]
async fn tidy_dataset_passes_without_approving(pool: PgPool) {
    let fake = common::fake("success", RUNNER_TIMEOUT);
    let path = evals::default_path(tidy::WORKFLOW);
    let dataset = evals::load(&path, tidy::WORKFLOW).expect("the shipped dataset loads");
    let report = evals::run(
        &pool,
        fake.runner.clone(),
        workflows::runtime(),
        &tidy::TidyEval,
        common::fast_config(),
        &dataset,
        "tidy_application.json",
        &options(Duration::from_secs(30)),
    )
    .await
    .expect("the eval runs");

    let text = report.text();
    assert!(report.passed(), "{text}");
    assert_eq!(report.totals.cases, 14, "{text}");
    assert_eq!(report.totals.pass_rate, 1.0);
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("pass rate: 14/14 (100.0%)")
    );
    assert!(
        report.cases.iter().all(|case| case.usage.calls == 0),
        "{text}"
    );

    let statuses: Vec<String> = sqlx::query_scalar(
        "SELECT status FROM pending_actions WHERE tool = 'draft_follow_up' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read pending actions");
    assert_eq!(statuses, ["pending"; 12]);
    let ran: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tool_calls WHERE tool = 'draft_follow_up' AND status = 'succeeded'",
    )
    .fetch_one(&pool)
    .await
    .expect("count draft calls");
    assert_eq!(ran, 0, "the harness never approves");
}

#[sqlx::test]
async fn memory_writer_suite_seeds_and_runs(pool: PgPool) {
    // Setup replaces every signal and memory with the case's own.
    let suite = memory_writer::MemoryWriterEval;
    sqlx::query("INSERT INTO memories (kind, topic, content) VALUES ('fact', 'skill', 'Old')")
        .execute(&pool)
        .await
        .expect("seed an old memory");
    let setup = json!({
        "memories": [{"name": "notice", "kind": "answer", "topic": "notice_period", "content": "Two weeks"}],
        "signals": [
            {"name": "edit", "kind": "approval", "subject": "draft_follow_up",
             "drafted": {"message": "a"}, "approved": {"message": "b"}},
            {"name": "plain", "kind": "approval", "subject": "draft_follow_up", "drafted": {"message": "a"}},
            {"name": "no", "kind": "rejection", "subject": "draft_follow_up", "drafted": {"message": "a"}},
            {"name": "why", "kind": "dismissal", "subject": "tidy.note", "drafted": {"note": "n"}, "reason": "Wrong role"}
        ]
    });
    let vars = suite.setup(&pool, &setup).await.expect("setup");
    let names: Vec<&String> = vars.keys().collect();
    assert_eq!(
        names,
        [
            "memory_notice",
            "signal_edit",
            "signal_no",
            "signal_plain",
            "signal_why"
        ]
    );
    /// `(kind, pending_action_id, review_item_id, changes, reason)`.
    type SignalRow = (
        String,
        Option<i64>,
        Option<i64>,
        Option<Value>,
        Option<String>,
    );
    let rows: Vec<SignalRow> = sqlx::query_as(
        "SELECT kind, pending_action_id, review_item_id, changes, reason \
             FROM memory_signals ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read signals");
    assert_eq!(
        rows,
        [
            (
                "approval".into(),
                Some(1),
                None,
                Some(json!([{"op": "replace", "path": "/message", "before": "a", "after": "b"}])),
                None
            ),
            ("approval".into(), Some(2), None, Some(json!([])), None),
            ("rejection".into(), Some(3), None, None, None),
            (
                "dismissal".into(),
                None,
                Some(4),
                None,
                Some("Wrong role".into())
            ),
        ]
    );
    let contents: Vec<String> = sqlx::query_scalar("SELECT content FROM memories")
        .fetch_all(&pool)
        .await
        .expect("read memories");
    assert_eq!(contents, ["Two weeks"]);
    assert!(
        suite
            .setup(&pool, &json!({"signals": [{"name": "x", "kind": "rejection", "subject": "s", "drafted": {}, "reason": "r"}]}))
            .await
            .is_err(),
        "only a dismissal has a reason"
    );

    // The shipped "no signals" case passes through the real worker without a call.
    let fake = common::fake("success", RUNNER_TIMEOUT);
    let path = evals::default_path(memory_writer::WORKFLOW);
    let dataset = evals::load(&path, memory_writer::WORKFLOW).expect("the shipped dataset loads");
    let report = evals::run(
        &pool,
        fake.runner.clone(),
        workflows::runtime(),
        &suite,
        common::fast_config(),
        &dataset,
        "memory_writer.json",
        &Options {
            cases: vec!["no signals makes no call".into()],
            case_timeout: Duration::from_secs(30),
        },
    )
    .await
    .expect("the eval runs");
    let text = report.text();
    assert!(report.passed(), "{text}");
    assert_eq!(report.totals.cases, 1, "{text}");
    assert_eq!(report.cases[0].usage.calls, 0, "{text}");
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("count {table}: {err}"))
}

#[sqlx::test]
async fn classify_email_suite_seeds_without_queuing(pool: PgPool) {
    let suite = classify_email::ClassifyEmailEval;

    // The files are ingested in order into one thread, and no run is queued.
    let setup =
        json!({"emails": ["emails/recruiter-outreach.eml", "emails/recruiter-outlook-reply.eml"]});
    let vars = suite.setup(&pool, &setup).await.expect("setup");
    let reply: i64 = sqlx::query_scalar(
        "SELECT id FROM emails WHERE provider_message_id = 'recruiter-outlook-reply'",
    )
    .fetch_one(&pool)
    .await
    .expect("the reply is stored");
    assert_eq!(Value::Object(vars), json!({"email_id": reply}));
    let threads: i64 = sqlx::query_scalar("SELECT count(DISTINCT thread_id) FROM emails")
        .fetch_one(&pool)
        .await
        .expect("count threads");
    assert_eq!((count(&pool, "emails").await, threads), (2, 1));
    for table in ["agent_runs", "jobs", "email_classifications"] {
        assert_eq!(count(&pool, table).await, 0, "{table}");
    }

    // A later setup starts from its own emails only.
    sqlx::query(
        "INSERT INTO email_classifications (email_id, input_key, classifier_version) \
         VALUES ($1, 'k', 1)",
    )
    .bind(reply)
    .execute(&pool)
    .await
    .expect("seed a classification");
    suite
        .setup(&pool, &json!({"emails": ["classify/offer.eml"]}))
        .await
        .expect("setup");
    assert_eq!(count(&pool, "emails").await, 1);
    assert_eq!(count(&pool, "email_threads").await, 1);
    assert_eq!(count(&pool, "email_classifications").await, 0);

    for bad in [
        json!(null),
        json!({}),
        json!({"emails": []}),
        json!({"emails": ["emails/ashby-rejection.eml"], "extra": 1}),
        json!({"emails": ["../Cargo.toml"]}),
        json!({"emails": ["emails/../../Cargo.toml"]}),
        json!({"emails": ["/etc/hosts"]}),
        json!({"emails": ["emails/missing.eml"]}),
        json!({"emails": ["emails"]}),
    ] {
        assert!(suite.setup(&pool, &bad).await.is_err(), "{bad}");
    }

    // A case run through the harness scores the fake's answer against its labels.
    let fake = common::fake("classify_email", RUNNER_TIMEOUT);
    let path = evals::default_path(classify_email::WORKFLOW);
    let dataset = evals::load(&path, classify_email::WORKFLOW).expect("the shipped dataset loads");
    let report = evals::run(
        &pool,
        fake.runner.clone(),
        workflows::runtime(),
        &suite,
        common::fast_config(),
        &dataset,
        "classify_email.json",
        &Options {
            cases: vec!["recruiter-outreach".into(), "ashby-rejection".into()],
            case_timeout: Duration::from_secs(30),
        },
    )
    .await
    .expect("the eval runs");
    let text = report.text();
    let results: Vec<(&str, CaseResult)> = report
        .cases
        .iter()
        .map(|case| (case.name.as_str(), case.result))
        .collect();
    assert_eq!(
        results,
        [
            ("ashby-rejection", CaseResult::Fail),
            ("recruiter-outreach", CaseResult::Pass)
        ],
        "{text}"
    );
    assert_eq!(
        report.cases[0].failures,
        [
            "output.class: expected \"rejection\", got \"availability_request\"",
            "output.company: expected \"Vandelay Industries\", got \"Soylent\""
        ],
        "{text}"
    );
    for case in &report.cases {
        assert_eq!(case.usage.calls, 1, "{text}");
    }
    assert_eq!(count(&pool, "agent_runs").await, 2, "one run per case");
}

#[sqlx::test]
async fn model_usage_is_attributed_per_case(pool: PgPool) {
    let case = |name: &str| json!({"name": name, "input": {}, "expect": {"status": "succeeded", "output": {"text": "pong"}}});
    let report = run_ask(
        &pool,
        "success",
        common::fast_config(),
        json!([case("first"), case("second")]),
        Duration::from_secs(30),
    )
    .await;

    assert!(report.passed(), "{}", report.text());
    for case in &report.cases {
        let usage = &case.usage;
        assert_eq!(
            (usage.calls, usage.input_tokens, usage.output_tokens),
            (1, 3978, 4),
            "{}",
            case.name
        );
        assert!((usage.cost_usd - 0.00067409).abs() < 1e-12, "{usage:?}");
        assert!(usage.call_ms > 0, "{usage:?}");
    }
    let totals = &report.totals.usage;
    assert_eq!(
        (totals.calls, totals.input_tokens, totals.output_tokens),
        (2, 7956, 8)
    );
    assert!(
        (totals.cost_usd - 2.0 * 0.00067409).abs() < 1e-12,
        "{totals:?}"
    );
}

#[sqlx::test]
async fn usage_limit_backs_off_then_passes(pool: PgPool) {
    let config = waypoint_api::jobs::Config {
        usage_limit_pause: Duration::from_secs(1),
        ..common::fast_config()
    };
    let report = run_ask(
        &pool,
        "usage_limit_once",
        config,
        json!([{"name": "waits out the limit", "input": {},
                "expect": {"status": "succeeded", "output": {"text": "pong"}}}]),
        Duration::from_secs(30),
    )
    .await;

    assert!(report.passed(), "{}", report.text());
    let case = &report.cases[0];
    assert_eq!((case.usage.calls, case.usage.usage_limit_hits), (2, 1));
    assert_eq!(case.usage.output_tokens, 4);
    assert!(case.wall_ms >= 1000, "the queue paused: {}ms", case.wall_ms);
}

#[sqlx::test]
async fn failures_timeouts_and_skips(pool: PgPool) {
    let report = run_ask(
        &pool,
        "hang",
        common::fast_config(),
        json!([
            {"name": "passes", "input": {"call": false},
             "expect": {"status": "succeeded", "output": {"text": null}}},
            {"name": "labels differ", "input": {"call": false},
             "expect": {"status": "succeeded", "output": {"text": "pong"}, "actions": [{"tool": "x"}]}},
            {"name": "hangs", "input": {}, "expect": {"status": "succeeded"}},
            {"name": "after the hang", "input": {"call": false}, "expect": {"status": "succeeded"}}
        ]),
        Duration::from_secs(1),
    )
    .await;

    let results: Vec<CaseResult> = report.cases.iter().map(|case| case.result).collect();
    assert_eq!(
        results,
        [
            CaseResult::Pass,
            CaseResult::Fail,
            CaseResult::Error,
            CaseResult::Skipped
        ],
        "{}",
        report.text()
    );
    assert_eq!(
        report.cases[1].failures,
        [
            "output.text: expected \"pong\", got null",
            "actions: expected 1 item(s), got 0"
        ]
    );
    let hang = &report.cases[2];
    assert_eq!(hang.error.as_deref(), Some("did not settle within 1s"));
    assert_eq!(hang.status, Some(RunStatus::Running));
    assert_eq!(report.cases[3].run_id, None, "a skipped case starts no run");
    assert_eq!(report.totals.pass_rate, 0.25);
    assert_eq!(
        (
            report.totals.failed,
            report.totals.errors,
            report.totals.skipped
        ),
        (1, 1, 1)
    );
    let text = report.text();
    assert!(
        text.contains("- output.text: expected \"pong\", got null"),
        "{text}"
    );
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("pass rate: 1/4 (25.0%)"),
        "{text}"
    );
}

/// Whether `name` is `waypoint_eval_<digits>_<digits>`.
fn scratch_shaped(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("waypoint_eval_") else {
        return false;
    };
    let parts: Vec<&str> = rest.split('_').collect();
    parts.len() == 2
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

async fn database_exists(admin: &PgPool, name: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(admin)
        .await
        .expect("read pg_database")
}

#[tokio::test]
async fn scratch_database_is_created_migrated_and_dropped() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is set for the tests");
    let admin = PgPool::connect(&url)
        .await
        .expect("connect to DATABASE_URL");

    let scratch = ScratchDb::create(&url)
        .await
        .expect("create the scratch db");
    let name = scratch.name().to_owned();
    assert!(scratch_shaped(&name), "{name}");
    assert!(database_exists(&admin, &name).await);
    let migrated: bool = sqlx::query_scalar("SELECT to_regclass('agent_runs') IS NOT NULL")
        .fetch_one(scratch.pool())
        .await
        .expect("query the scratch db");
    assert!(migrated, "agent_runs exists");
    let paused: bool = sqlx::query_scalar("SELECT paused FROM kill_switch")
        .fetch_one(scratch.pool())
        .await
        .expect("read the kill switch");
    assert!(!paused, "the kill switch starts off");

    scratch.drop().await.expect("drop the scratch db");
    assert!(!database_exists(&admin, &name).await);
}
