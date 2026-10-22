//! Agent runs: the tables, starting and resuming runs, step execution through the worker,
//! tool invocations and the read-only `/runs` routes. Model calls go to `fake-claude.sh`.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
use waypoint_api::claude::{ClaudeRequest, ClaudeRunner};
use waypoint_api::events::{Event, EventBus, Subscriber};
use waypoint_api::jobs::{self, Wake, Worker};
use waypoint_api::runs::{self, Run, RunStatus, Runtime, StepContext, StepError, Workflow};
use waypoint_api::tools::{Permission, Registry, Tool, ToolContext, ToolError};

const TIMEOUT: Duration = Duration::from_secs(10);

/// A worker running against the fake in `mode` with `runtime`, plus what a test needs to
/// drive it.
struct Harness {
    bus: EventBus,
    wake: Wake,
    pool: PgPool,
    _fake: common::Fake,
    _stop: watch::Sender<bool>,
}

impl Harness {
    /// Starts a worker. Subscribe to `bus` before starting runs to see every event.
    fn start(
        pool: &PgPool,
        mode: &str,
        config: jobs::Config,
        runtime: Runtime,
    ) -> (Self, Subscriber) {
        let fake = common::fake(mode, TIMEOUT);
        let bus = EventBus::new();
        let subscriber = bus.subscribe();
        let wake = Wake::default();
        let (stop, stopped) = watch::channel(false);
        Worker {
            pool: pool.clone(),
            runner: fake.runner.clone(),
            bus: bus.clone(),
            config,
            wake: wake.0.clone(),
            runtime,
        }
        .spawn(stopped);
        let harness = Self {
            bus,
            wake,
            pool: pool.clone(),
            _fake: fake,
            _stop: stop,
        };
        (harness, subscriber)
    }

    async fn start_run(&self, runtime: &Runtime, workflow: &str, input: Value) -> Run {
        runs::start(&self.pool, &self.bus, &self.wake, runtime, workflow, input)
            .await
            .expect("start the run")
    }
}

/// Collects events until job `id` succeeds or fails.
async fn until_finished(subscriber: &mut Subscriber, id: i64) -> Vec<Event> {
    common::events(subscriber, TIMEOUT, |event| {
        matches!(event, Event::JobSucceeded { job } | Event::JobFailed { job } if job.id == id)
    })
    .await
}

/// A step row as `(name, status, attempts, input, output, error)`.
type StepRow = (
    String,
    String,
    i32,
    Option<Value>,
    Option<Value>,
    Option<String>,
);

async fn steps(pool: &PgPool, run_id: i64) -> Vec<StepRow> {
    sqlx::query_as(
        "SELECT name, status, attempts, input, output, error FROM run_steps \
         WHERE run_id = $1 ORDER BY ordinal",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read steps")
}

async fn run(pool: &PgPool, id: i64) -> Run {
    runs::list_runs(pool, 200, None)
        .await
        .expect("list runs")
        .into_iter()
        .find(|run| run.id == id)
        .expect("the run exists")
}

async fn job_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(pool)
        .await
        .expect("count jobs")
}

/// Two steps, `first` then `second`, each wrapping its input. `second` fails while
/// `second_failures` is above zero, with the error `failure` builds.
struct TwoStep {
    first_calls: Arc<AtomicUsize>,
    second_calls: Arc<AtomicUsize>,
    second_failures: Arc<AtomicUsize>,
    failure: fn(String) -> StepError,
}

impl TwoStep {
    fn new(second_failures: usize, failure: fn(String) -> StepError) -> Self {
        Self {
            first_calls: Arc::default(),
            second_calls: Arc::default(),
            second_failures: Arc::new(AtomicUsize::new(second_failures)),
            failure,
        }
    }

    fn calls(&self) -> (usize, usize) {
        (
            self.first_calls.load(Ordering::SeqCst),
            self.second_calls.load(Ordering::SeqCst),
        )
    }
}

#[rocket::async_trait]
impl Workflow for TwoStep {
    fn name(&self) -> &'static str {
        "two_step"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["first", "second"]
    }

    async fn run_step(
        &self,
        _ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        match step {
            "first" => {
                self.first_calls.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"first": input}))
            }
            "second" => {
                self.second_calls.fetch_add(1, Ordering::SeqCst);
                // Decrement the remaining failures if any are left. A plain
                // compare_exchange loop, since `fetch_update` is deprecated on
                // newer toolchains and its replacement is missing on older ones.
                let mut left = self.second_failures.load(Ordering::SeqCst);
                let failing = loop {
                    if left == 0 {
                        break false;
                    }
                    match self.second_failures.compare_exchange(
                        left,
                        left - 1,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break true,
                        Err(current) => left = current,
                    }
                };
                if failing {
                    return Err((self.failure)("second broke".into()));
                }
                Ok(json!({"second": input}))
            }
            other => Err(StepError::Fail(format!("no step {other}"))),
        }
    }
}

/// Builds a runtime from shared workflows, so a test keeps its handle on the counters.
fn runtime_of(workflows: Vec<Arc<dyn Workflow>>, tools: Registry) -> Runtime {
    Runtime::new(workflows, tools).expect("valid runtime")
}

#[sqlx::test]
async fn run_tables_enforce_constraints(pool: PgPool) {
    let (run_id,): (i64,) =
        sqlx::query_as("INSERT INTO agent_runs (workflow, input) VALUES ('w', '{}') RETURNING id")
            .fetch_one(&pool)
            .await
            .expect("insert a run");
    let (step_id,): (i64,) = sqlx::query_as(
        "INSERT INTO run_steps (run_id, ordinal, name) VALUES ($1, 0, 'a') RETURNING id",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .expect("insert a step");
    sqlx::query(
        "INSERT INTO tool_calls (run_step_id, tool, permission, status, input) \
         VALUES ($1, 't', 'read', 'running', '{}')",
    )
    .bind(step_id)
    .execute(&pool)
    .await
    .expect("insert a tool call");

    let refused = [
        (
            "a bad run status",
            "INSERT INTO agent_runs (workflow, input, status) VALUES ('w', '{}', 'paused')"
                .to_string(),
        ),
        (
            "a bad step status",
            format!(
                "INSERT INTO run_steps (run_id, ordinal, name, status) VALUES ({run_id}, 1, 'b', 'done')"
            ),
        ),
        (
            "a duplicate ordinal",
            format!("INSERT INTO run_steps (run_id, ordinal, name) VALUES ({run_id}, 0, 'b')"),
        ),
        (
            "a negative ordinal",
            format!("INSERT INTO run_steps (run_id, ordinal, name) VALUES ({run_id}, -1, 'b')"),
        ),
        (
            "a dangling step claude_call_id",
            format!(
                "INSERT INTO run_steps (run_id, ordinal, name, claude_call_id) VALUES ({run_id}, 1, 'b', 999999)"
            ),
        ),
        (
            "a bad tool call status",
            format!(
                "INSERT INTO tool_calls (run_step_id, tool, status, input) VALUES ({step_id}, 't', 'ok', '{{}}')"
            ),
        ),
        (
            "a bad permission",
            format!(
                "INSERT INTO tool_calls (run_step_id, tool, permission, status, input) VALUES ({step_id}, 't', 'admin', 'running', '{{}}')"
            ),
        ),
        (
            "a dangling tool call claude_call_id",
            format!(
                "INSERT INTO tool_calls (run_step_id, tool, status, input, claude_call_id) VALUES ({step_id}, 't', 'running', '{{}}', 999999)"
            ),
        ),
        (
            "negative run tokens",
            format!("UPDATE agent_runs SET input_tokens = -1 WHERE id = {run_id}"),
        ),
    ];
    for (what, sql) in refused {
        assert!(
            sqlx::query(&sql).execute(&pool).await.is_err(),
            "{what} is refused"
        );
    }

    // Deleting a run deletes its steps and their tool calls.
    sqlx::query("DELETE FROM agent_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("delete the run");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM tool_calls")
        .fetch_one(&pool)
        .await
        .expect("count tool calls");
    assert_eq!(left, 0);
}

#[sqlx::test]
async fn start_creates_run_steps_and_job(pool: PgPool) {
    let runtime = runtime_of(
        vec![Arc::new(TwoStep::new(0, StepError::Fail))],
        Registry::default(),
    );
    let bus = EventBus::new();
    let wake = Wake::default();

    let err = runs::start(&pool, &bus, &wake, &runtime, "nope", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, runs::StartError::UnknownWorkflow(ref name) if name == "nope"));
    for table in ["agent_runs", "run_steps", "jobs"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 0, "an unknown workflow wrote to {table}");
    }

    let input = json!({"posting": 7});
    let run = runs::start(&pool, &bus, &wake, &runtime, "two_step", input.clone())
        .await
        .expect("start");
    assert_eq!(run.workflow, "two_step");
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.input, input);
    assert_eq!((run.output.clone(), run.error.clone()), (None, None));
    assert_eq!(
        (run.input_tokens, run.output_tokens, run.cost_usd),
        (0, 0, 0.0)
    );

    let rows: Vec<(i32, String, String, i32)> = sqlx::query_as(
        "SELECT ordinal, name, status, attempts FROM run_steps WHERE run_id = $1 ORDER BY ordinal",
    )
    .bind(run.id)
    .fetch_all(&pool)
    .await
    .expect("read steps");
    assert_eq!(
        rows,
        [
            (0, "first".to_string(), "pending".to_string(), 0),
            (1, "second".to_string(), "pending".to_string(), 0),
        ]
    );

    let job_id = run.job_id.expect("job_id is set");
    let job = jobs::get(&pool, job_id).await.expect("get").expect("job");
    assert_eq!(job.kind, jobs::AGENT_RUN);
    assert_eq!(job.status, "queued");
    assert_eq!(job.payload, json!({"run_id": run.id}));
    assert_eq!(job_count(&pool).await, 1);
}

#[sqlx::test]
async fn run_executes_steps_in_order(pool: PgPool) {
    let workflow = Arc::new(TwoStep::new(0, StepError::Fail));
    let runtime = runtime_of(vec![workflow.clone()], Registry::default());
    let (harness, mut subscriber) =
        Harness::start(&pool, "success", common::fast_config(), runtime.clone());
    let input = json!({"posting": 7});
    let started = harness.start_run(&runtime, "two_step", input.clone()).await;
    let job_id = started.job_id.expect("job_id");
    until_finished(&mut subscriber, job_id).await;

    let first_out = json!({"first": input});
    let second_out = json!({"second": first_out});
    assert_eq!(
        steps(&pool, started.id).await,
        [
            (
                "first".into(),
                "succeeded".into(),
                1,
                Some(input.clone()),
                Some(first_out.clone()),
                None
            ),
            (
                "second".into(),
                "succeeded".into(),
                1,
                Some(first_out),
                Some(second_out.clone()),
                None
            ),
        ]
    );
    let done = run(&pool, started.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(done.output, Some(second_out));
    assert!(done.started_at.is_some() && done.finished_at.is_some());
    let job = jobs::get(&pool, job_id).await.expect("get").expect("job");
    assert_eq!(job.status, "succeeded");
    assert_eq!(job.result, Some(json!({"run_id": started.id})));
    assert_eq!(workflow.calls(), (1, 1));
}

#[sqlx::test]
async fn retry_skips_succeeded_steps(pool: PgPool) {
    let workflow = Arc::new(TwoStep::new(1, StepError::Retry));
    let runtime = runtime_of(vec![workflow.clone()], Registry::default());
    // A slow first retry leaves time to look at the run while it waits.
    let config = jobs::Config {
        retry_base: Duration::from_millis(800),
        retry_cap: Duration::from_secs(2),
        ..common::fast_config()
    };
    let (harness, mut subscriber) = Harness::start(&pool, "success", config, runtime.clone());
    let started = harness.start_run(&runtime, "two_step", json!({})).await;
    let job_id = started.job_id.expect("job_id");

    common::events(
        &mut subscriber,
        TIMEOUT,
        |event| matches!(event, Event::JobRetrying { job } if job.id == job_id),
    )
    .await;
    let waiting = run(&pool, started.id).await;
    assert_eq!(waiting.status, RunStatus::Queued);
    let error = waiting.error.expect("the run's error is set");
    assert!(error.contains("second broke"), "{error}");
    assert!(waiting.finished_at.is_none());

    until_finished(&mut subscriber, job_id).await;
    let done = run(&pool, started.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(done.error, None);
    let attempts: Vec<(String, String, i32)> = steps(&pool, started.id)
        .await
        .into_iter()
        .map(|(name, status, attempts, ..)| (name, status, attempts))
        .collect();
    assert_eq!(
        attempts,
        [
            ("first".into(), "succeeded".into(), 1),
            ("second".into(), "succeeded".into(), 2),
        ]
    );
    assert_eq!(workflow.calls(), (1, 2));
}

#[sqlx::test]
async fn resume_reruns_only_unfinished_steps(pool: PgPool) {
    let workflow = Arc::new(TwoStep::new(1, StepError::Fail));
    let runtime = runtime_of(vec![workflow.clone()], Registry::default());
    let (harness, mut subscriber) =
        Harness::start(&pool, "success", common::fast_config(), runtime.clone());
    let started = harness.start_run(&runtime, "two_step", json!({})).await;
    let first_job = started.job_id.expect("job_id");
    until_finished(&mut subscriber, first_job).await;

    let failed = run(&pool, started.id).await;
    assert_eq!(failed.status, RunStatus::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .unwrap_or("")
            .contains("second broke")
    );
    assert!(failed.finished_at.is_some());
    let job = jobs::get(&pool, first_job)
        .await
        .expect("get")
        .expect("job");
    assert_eq!((job.status.as_str(), job.attempts), ("failed", 1));

    let jobs_before = job_count(&pool).await;
    let resumed = |id| runs::resume(&harness.pool, &harness.bus, &harness.wake, id);
    assert_eq!(resumed(started.id + 1000).await.expect("resume"), None);
    let again = resumed(started.id)
        .await
        .expect("resume")
        .expect("a failed run resumes");
    assert_eq!(again.status, RunStatus::Queued);
    assert_eq!(again.finished_at, None);
    let second_job = again.job_id.expect("job_id");
    assert_ne!(second_job, first_job);
    assert_eq!(
        resumed(started.id).await.expect("resume"),
        None,
        "a queued run doesn't resume"
    );
    assert_eq!(job_count(&pool).await, jobs_before + 1);

    until_finished(&mut subscriber, second_job).await;
    let done = run(&pool, started.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(done.job_id, Some(second_job));
    assert_eq!(
        workflow.calls(),
        (1, 2),
        "the succeeded first step is skipped"
    );
    let attempts: Vec<i32> = steps(&pool, started.id)
        .await
        .into_iter()
        .map(|(_, _, attempts, ..)| attempts)
        .collect();
    assert_eq!(attempts, [1, 2]);

    assert_eq!(
        resumed(started.id).await.expect("resume"),
        None,
        "a succeeded run doesn't resume"
    );
    assert_eq!(job_count(&pool).await, jobs_before + 1);
}

#[sqlx::test]
async fn orphaned_step_reruns_after_restart(pool: PgPool) {
    let workflow = Arc::new(TwoStep::new(0, StepError::Fail));
    let runtime = runtime_of(vec![workflow.clone()], Registry::default());
    let started = runs::start(
        &pool,
        &EventBus::new(),
        &Wake::default(),
        &runtime,
        "two_step",
        json!({}),
    )
    .await
    .expect("start");
    let job_id = started.job_id.expect("job_id");
    // A crash mid-step: the job, the run and the first step were left running.
    for sql in [
        "UPDATE jobs SET status = 'running', attempts = 1 WHERE id = $1",
        "UPDATE agent_runs SET status = 'running', started_at = now() WHERE job_id = $1",
        "UPDATE run_steps SET status = 'running', attempts = 1, input = '{}' \
         WHERE ordinal = 0 AND run_id = (SELECT id FROM agent_runs WHERE job_id = $1)",
    ] {
        sqlx::query(sql)
            .bind(job_id)
            .execute(&pool)
            .await
            .expect("simulate a crash");
    }

    let (_harness, mut subscriber) =
        Harness::start(&pool, "success", common::fast_config(), runtime.clone());
    until_finished(&mut subscriber, job_id).await;

    let done = run(&pool, started.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    let attempts: Vec<(String, i32)> = steps(&pool, started.id)
        .await
        .into_iter()
        .map(|(_, status, attempts, ..)| (status, attempts))
        .collect();
    assert_eq!(attempts, [("succeeded".into(), 2), ("succeeded".into(), 1)]);
    assert_eq!(workflow.calls(), (1, 1));
}

/// Two steps that call the model: `ask` twice directly, `summarize` once through the
/// `summarize` tool.
struct Asking;

#[rocket::async_trait]
impl Workflow for Asking {
    fn name(&self) -> &'static str {
        "asking"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["ask", "summarize"]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        step: &str,
        _input: Value,
    ) -> Result<Value, StepError> {
        match step {
            "ask" => {
                let mut texts = Vec::new();
                for purpose in ["test.ask.1", "test.ask.2"] {
                    let response = ctx.call(&request(purpose)).await?;
                    texts.push(response.text);
                }
                Ok(json!({"texts": texts}))
            }
            _ => Ok(ctx.tool("summarize", json!({})).await?),
        }
    }
}

fn request(purpose: &str) -> ClaudeRequest {
    ClaudeRequest {
        prompt: "ping".into(),
        purpose: purpose.into(),
        model: None,
    }
}

/// A `read` tool that makes one model call.
struct Summarize;

#[rocket::async_trait]
impl Tool for Summarize {
    const NAME: &'static str = "summarize";
    const DESCRIPTION: &'static str = "Asks the model once";
    const PERMISSION: Permission = Permission::Read;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object", "additionalProperties": false})
    }

    fn output_schema() -> Value {
        json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]})
    }

    async fn call(&self, ctx: &mut ToolContext<'_>, _input: Value) -> Result<Value, ToolError> {
        let response = ctx.call(&request("test.summarize")).await?;
        Ok(json!({"text": response.text}))
    }
}

fn asking_runtime() -> Runtime {
    let mut tools = Registry::default();
    tools.register(Summarize).expect("register summarize");
    runtime_of(vec![Arc::new(Asking)], tools)
}

#[sqlx::test]
async fn step_records_model_usage(pool: PgPool) {
    let runtime = asking_runtime();
    let (harness, mut subscriber) =
        Harness::start(&pool, "success", common::fast_config(), runtime.clone());
    let started = harness.start_run(&runtime, "asking", json!({})).await;
    until_finished(&mut subscriber, started.job_id.expect("job_id")).await;

    // What each successful call spends, read back from the call log.
    let calls: Vec<(i64, String, i64, i64, f64)> = sqlx::query_as(
        "SELECT id, purpose, \
         COALESCE(input_tokens, 0) + COALESCE(cache_creation_input_tokens, 0) \
         + COALESCE(cache_read_input_tokens, 0), \
         COALESCE(output_tokens, 0), COALESCE(cost_usd, 0) FROM claude_calls ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read claude_calls");
    let purposes: Vec<&str> = calls.iter().map(|call| call.1.as_str()).collect();
    assert_eq!(purposes, ["test.ask.1", "test.ask.2", "test.summarize"]);
    let (_, _, input, output, cost) = calls[0].clone();
    assert!(input > 0 && output > 0 && cost > 0.0, "{calls:?}");

    let rows: Vec<(String, i64, i64, f64, Option<i64>)> = sqlx::query_as(
        "SELECT status, input_tokens, output_tokens, cost_usd, claude_call_id FROM run_steps \
         WHERE run_id = $1 ORDER BY ordinal",
    )
    .bind(started.id)
    .fetch_all(&pool)
    .await
    .expect("read steps");
    assert_eq!(
        rows[0],
        (
            "succeeded".into(),
            2 * input,
            2 * output,
            2.0 * cost,
            Some(calls[1].0)
        ),
        "the step sums its calls and links the latest"
    );
    assert_eq!(
        rows[1],
        ("succeeded".into(), input, output, cost, None),
        "a tool's calls count against its step"
    );
    let tool_spend: (i64, i64, f64) =
        sqlx::query_as("SELECT input_tokens, output_tokens, cost_usd FROM tool_calls")
            .fetch_one(&pool)
            .await
            .expect("read the tool call");
    assert_eq!(tool_spend, (input, output, cost));

    let done = run(&pool, started.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(
        (done.input_tokens, done.output_tokens),
        (rows[0].1 + rows[1].1, rows[0].2 + rows[1].2)
    );
    assert!((done.cost_usd - (rows[0].3 + rows[1].3)).abs() < 1e-12);
}

#[sqlx::test]
async fn usage_limit_defers_run(pool: PgPool) {
    let runtime = asking_runtime();
    let (harness, mut subscriber) =
        Harness::start(&pool, "usage_limit", common::fast_config(), runtime.clone());
    let started = harness.start_run(&runtime, "asking", json!({})).await;
    let job_id = started.job_id.expect("job_id");
    common::events(
        &mut subscriber,
        TIMEOUT,
        |event| matches!(event, Event::JobDeferred { job } if job.id == job_id),
    )
    .await;

    let rows = steps(&pool, started.id).await;
    assert_eq!((rows[0].1.as_str(), rows[0].2), ("failed", 1));
    let error = rows[0].5.as_deref().expect("the step's error");
    assert!(error.contains("hit your limit"), "{error}");
    assert_eq!((rows[1].1.as_str(), rows[1].2), ("pending", 0));

    let waiting = run(&pool, started.id).await;
    assert_eq!(waiting.status, RunStatus::Queued);
    assert!(
        waiting
            .error
            .as_deref()
            .unwrap_or("")
            .contains("hit your limit")
    );
    assert_eq!(waiting.finished_at, None);
    assert_eq!((waiting.input_tokens, waiting.cost_usd), (0, 0.0));
    let job = jobs::get(&pool, job_id).await.expect("get").expect("job");
    assert_eq!(
        (job.status.as_str(), job.attempts),
        ("queued", 0),
        "the attempt is given back"
    );
}

#[sqlx::test]
async fn unknown_workflow_fails_run(pool: PgPool) {
    let runtime = runtime_of(
        vec![Arc::new(TwoStep::new(0, StepError::Fail))],
        Registry::default(),
    );
    // The worker's runtime doesn't have the workflow the run was started with.
    let (harness, mut subscriber) =
        Harness::start(&pool, "success", common::fast_config(), Runtime::default());
    let started = harness.start_run(&runtime, "two_step", json!({})).await;
    let job_id = started.job_id.expect("job_id");
    let events = until_finished(&mut subscriber, job_id).await;

    let failed = run(&pool, started.id).await;
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(
        run_updates(&events, started.id).last(),
        Some(&failed),
        "the failure is published"
    );
    assert!(failed.error.as_deref().unwrap_or("").contains("two_step"));
    assert!(failed.finished_at.is_some());
    let job = jobs::get(&pool, job_id).await.expect("get").expect("job");
    assert_eq!((job.status.as_str(), job.attempts), ("failed", 1));
    let statuses: Vec<String> = steps(&pool, started.id)
        .await
        .into_iter()
        .map(|(_, status, ..)| status)
        .collect();
    assert_eq!(statuses, ["pending", "pending"]);
}

/// Counts handler calls per tool.
#[derive(Clone, Default)]
struct ToolCounts(Arc<[AtomicUsize; 3]>);

impl ToolCounts {
    fn get(&self) -> [usize; 3] {
        [0, 1, 2].map(|i| self.0[i].load(Ordering::SeqCst))
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupInput {
    q: String,
}

/// A `read` tool that upper-cases `q`.
struct Lookup(ToolCounts);

#[rocket::async_trait]
impl Tool for Lookup {
    const NAME: &'static str = "lookup";
    const DESCRIPTION: &'static str = "Upper-cases a query";
    const PERMISSION: Permission = Permission::Read;
    type Input = LookupInput;
    type Output = Value;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"q": {"type": "string"}},
            "required": ["q"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({"type": "object", "properties": {"answer": {"type": "string"}}})
    }

    async fn call(
        &self,
        _ctx: &mut ToolContext<'_>,
        input: LookupInput,
    ) -> Result<Value, ToolError> {
        self.0.0[0].fetch_add(1, Ordering::SeqCst);
        Ok(json!({"answer": input.q.to_uppercase()}))
    }
}

/// A `write` tool whose handler always fails.
struct Broken(ToolCounts);

#[rocket::async_trait]
impl Tool for Broken {
    const NAME: &'static str = "broken";
    const DESCRIPTION: &'static str = "Always fails";
    const PERMISSION: Permission = Permission::Write;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object"})
    }

    fn output_schema() -> Value {
        json!({"type": "object"})
    }

    async fn call(&self, _ctx: &mut ToolContext<'_>, _input: Value) -> Result<Value, ToolError> {
        self.0.0[1].fetch_add(1, Ordering::SeqCst);
        Err(ToolError::Failed("the outbox is closed".into()))
    }
}

/// An `external` tool whose output breaks its own schema.
struct Liar(ToolCounts);

#[rocket::async_trait]
impl Tool for Liar {
    const NAME: &'static str = "liar";
    const DESCRIPTION: &'static str = "Returns the wrong shape";
    const PERMISSION: Permission = Permission::External;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object"})
    }

    fn output_schema() -> Value {
        json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]})
    }

    async fn call(&self, _ctx: &mut ToolContext<'_>, _input: Value) -> Result<Value, ToolError> {
        self.0.0[2].fetch_add(1, Ordering::SeqCst);
        Ok(json!({"n": "seven"}))
    }
}

/// One step that calls tools in order and reports each result or error, so it succeeds.
struct Tooling(Vec<(&'static str, Value)>);

#[rocket::async_trait]
impl Workflow for Tooling {
    fn name(&self) -> &'static str {
        "tooling"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["use_tools"]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        _step: &str,
        _input: Value,
    ) -> Result<Value, StepError> {
        let mut results = Vec::new();
        for (name, input) in &self.0 {
            results.push(match ctx.tool(name, input.clone()).await {
                Ok(output) => output,
                Err(err) => json!(err.to_string()),
            });
        }
        Ok(Value::Array(results))
    }
}

#[sqlx::test]
async fn tool_invocations_are_recorded(pool: PgPool) {
    let counts = ToolCounts::default();
    let mut tools = Registry::default();
    tools.register(Lookup(counts.clone())).expect("lookup");
    tools.register(Broken(counts.clone())).expect("broken");
    tools.register(Liar(counts.clone())).expect("liar");
    let workflow = Tooling(vec![
        ("lookup", json!({"q": "hi"})),
        ("nope", json!({"x": 1})),
        ("lookup", json!({"q": 5})),
        ("broken", json!({})),
        ("liar", json!({})),
    ]);
    let runtime = runtime_of(vec![Arc::new(workflow)], tools);
    let (harness, mut subscriber) =
        Harness::start(&pool, "success", common::fast_config(), runtime.clone());
    let started = harness.start_run(&runtime, "tooling", json!({})).await;
    until_finished(&mut subscriber, started.job_id.expect("job_id")).await;

    assert_eq!(
        counts.get(),
        [1, 1, 1],
        "unknown and invalid input never reach a handler"
    );
    type Row = (
        String,
        Option<String>,
        String,
        Value,
        Option<Value>,
        Option<String>,
        Option<i64>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT t.tool, t.permission, t.status, t.input, t.output, t.error, t.duration_ms \
         FROM tool_calls AS t JOIN run_steps AS s ON s.id = t.run_step_id \
         WHERE s.run_id = $1 ORDER BY t.id",
    )
    .bind(started.id)
    .fetch_all(&pool)
    .await
    .expect("read tool calls");
    /// `(tool, permission, status, input, output, error)`.
    type Expected = (
        &'static str,
        Option<&'static str>,
        &'static str,
        Value,
        Option<Value>,
        Option<&'static str>,
    );
    let expected: [Expected; 5] = [
        (
            "lookup",
            Some("read"),
            "succeeded",
            json!({"q": "hi"}),
            Some(json!({"answer": "HI"})),
            None,
        ),
        (
            "nope",
            None,
            "unknown_tool",
            json!({"x": 1}),
            None,
            Some("no tool has this name"),
        ),
        (
            "lookup",
            Some("read"),
            "invalid_input",
            json!({"q": 5}),
            None,
            Some("invalid tool input: $.q: expected string"),
        ),
        (
            "broken",
            Some("write"),
            "failed",
            json!({}),
            None,
            Some("the tool failed: the outbox is closed"),
        ),
        (
            "liar",
            Some("external"),
            "invalid_output",
            json!({}),
            Some(json!({"n": "seven"})),
            Some("invalid tool output: $.n: expected integer"),
        ),
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, want) in rows.iter().zip(expected) {
        let (tool, permission, status, input, output, error, duration) = row;
        assert_eq!(
            (
                tool.as_str(),
                permission.as_deref(),
                status.as_str(),
                input,
                output,
                error.as_deref()
            ),
            (want.0, want.1, want.2, &want.3, &want.4, want.5),
        );
        assert!(duration.is_some_and(|ms| ms >= 0), "{tool} has a duration");
    }

    let done = run(&pool, started.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    let output = done.output.expect("output");
    assert_eq!(output[0], json!({"answer": "HI"}));
    assert_eq!(output[1], json!("no tool has this name"));
}

/// The `run.updated` events for run `id`, in order.
fn run_updates(events: &[Event], id: i64) -> Vec<Run> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::RunUpdated { run } if run.id == id => Some(run.clone()),
            _ => None,
        })
        .collect()
}

#[sqlx::test]
async fn run_updated_events_follow_the_run(pool: PgPool) {
    let mut tools = Registry::default();
    tools
        .register(Lookup(ToolCounts::default()))
        .expect("lookup");
    let workflow = Tooling(vec![("lookup", json!({"q": "hi"}))]);
    let runtime = runtime_of(vec![Arc::new(workflow)], tools);
    let (harness, mut subscriber) =
        Harness::start(&pool, "success", common::fast_config(), runtime.clone());
    let started = harness.start_run(&runtime, "tooling", json!({})).await;
    let events = until_finished(&mut subscriber, started.job_id.expect("job_id")).await;

    let updates = run_updates(&events, started.id);
    let statuses: Vec<RunStatus> = updates.iter().map(|run| run.status).collect();
    use RunStatus::{Queued, Running, Succeeded};
    assert_eq!(
        statuses,
        [Queued, Running, Running, Running, Running, Succeeded],
        "start, run running, step running, tool returned, step succeeded, run succeeded"
    );
    assert_eq!(updates.last(), Some(&run(&pool, started.id).await));
}

/// A client for the API whose worker finds no ready jobs: tests seed runs first, then mark
/// their jobs finished.
async fn client(pool: &PgPool) -> Client {
    sqlx::query("UPDATE jobs SET status = 'succeeded'")
        .execute(pool)
        .await
        .expect("park the jobs");
    let rocket = common::app(
        pool.clone(),
        ClaudeRunner::new("claude", Duration::from_secs(1)),
        common::fast_config(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
}

async fn get_json(client: &Client, path: &str) -> Value {
    let response = client
        .get(path.to_string())
        .header(Header::new("Host", common::HOST))
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok, "{path}");
    response.into_json().await.expect("json body")
}

fn ids(body: &Value) -> Vec<i64> {
    body.as_array()
        .expect("an array")
        .iter()
        .map(|run| run["id"].as_i64().expect("id"))
        .collect()
}

async fn seed_runs(pool: &PgPool, n: usize) -> Vec<i64> {
    let runtime = runtime_of(
        vec![Arc::new(TwoStep::new(0, StepError::Fail))],
        Registry::default(),
    );
    let (bus, wake) = (EventBus::new(), Wake::default());
    let mut ids = Vec::new();
    for i in 0..n {
        let run = runs::start(pool, &bus, &wake, &runtime, "two_step", json!({"i": i}))
            .await
            .expect("start");
        ids.push(run.id);
    }
    ids
}

#[sqlx::test]
async fn list_runs_paginates(pool: PgPool) {
    {
        let client = client(&pool).await;
        assert_eq!(get_json(&client, "/runs").await, json!([]));
    }

    let mut seeded = seed_runs(&pool, 5).await;
    seeded.reverse();
    let client = client(&pool).await;

    let all = get_json(&client, "/runs").await;
    assert_eq!(ids(&all), seeded, "newest first");
    assert_eq!(all[0]["status"], "queued");
    assert_eq!(all[0]["workflow"], "two_step");

    assert_eq!(ids(&get_json(&client, "/runs?limit=2").await), seeded[..2]);
    let next = format!("/runs?limit=2&before={}", seeded[1]);
    assert_eq!(ids(&get_json(&client, &next).await), seeded[2..4]);
    let last = format!("/runs?before={}", seeded[4]);
    assert_eq!(get_json(&client, &last).await, json!([]));
    assert_eq!(
        ids(&get_json(&client, "/runs?limit=0").await),
        seeded[..1],
        "clamped up to 1"
    );
    assert_eq!(ids(&get_json(&client, "/runs?limit=-3").await), seeded[..1]);

    sqlx::query(
        "INSERT INTO agent_runs (workflow, input) SELECT 'bulk', '{}' FROM generate_series(1, 200)",
    )
    .execute(&pool)
    .await
    .expect("insert many runs");
    let page = get_json(&client, "/runs?limit=1000").await;
    assert_eq!(
        page.as_array().map(Vec::len),
        Some(200),
        "clamped down to 200"
    );
    assert_eq!(
        get_json(&client, "/runs").await.as_array().map(Vec::len),
        Some(50),
        "50 by default"
    );
}

#[sqlx::test]
async fn show_run_includes_steps_and_tool_calls(pool: PgPool) {
    let id = seed_runs(&pool, 1).await[0];
    let step_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM run_steps WHERE run_id = $1 ORDER BY ordinal")
            .bind(id)
            .fetch_all(&pool)
            .await
            .expect("step ids");
    sqlx::query(
        "UPDATE run_steps SET status = 'succeeded', attempts = 1, input = '{}', \
         output = '{\"a\": 1}' WHERE id = $1",
    )
    .bind(step_ids[0])
    .execute(&pool)
    .await
    .expect("finish the first step");
    // The second step's call is inserted first, so grouping by step is visible.
    let mut call_ids = Vec::new();
    for (step, tool, permission, status) in [
        (step_ids[1], "later", Some("external"), "running"),
        (step_ids[0], "lookup", Some("read"), "succeeded"),
        (step_ids[0], "nope", None, "unknown_tool"),
    ] {
        let call: i64 = sqlx::query_scalar(
            "INSERT INTO tool_calls (run_step_id, tool, permission, status, input) \
             VALUES ($1, $2, $3, $4, '{\"q\": 1}') RETURNING id",
        )
        .bind(step)
        .bind(tool)
        .bind(permission)
        .bind(status)
        .fetch_one(&pool)
        .await
        .expect("insert a tool call");
        call_ids.push(call);
    }
    let client = client(&pool).await;

    let body = get_json(&client, &format!("/runs/{id}")).await;
    assert_eq!(body["id"], id);
    assert_eq!(body["workflow"], "two_step");
    assert_eq!(body["input"], json!({"i": 0}));
    assert!(body["output"].is_null() && body.get("output").is_some());
    let steps = body["steps"].as_array().expect("steps");
    let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["first", "second"]);
    assert_eq!(steps[0]["ordinal"], 0);
    assert_eq!(steps[0]["status"], "succeeded");
    assert_eq!(steps[0]["output"], json!({"a": 1}));
    assert_eq!(steps[1]["status"], "pending");
    assert!(steps[1].get("claude_call_id").is_some_and(Value::is_null));

    let calls = |step: &Value| -> Vec<(i64, String, Value, String)> {
        step["tool_calls"]
            .as_array()
            .expect("tool_calls")
            .iter()
            .map(|call| {
                (
                    call["id"].as_i64().unwrap(),
                    call["tool"].as_str().unwrap().to_string(),
                    call["permission"].clone(),
                    call["status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    assert_eq!(
        calls(&steps[0]),
        [
            (
                call_ids[1],
                "lookup".into(),
                json!("read"),
                "succeeded".into()
            ),
            (
                call_ids[2],
                "nope".into(),
                Value::Null,
                "unknown_tool".into()
            ),
        ]
    );
    assert_eq!(
        calls(&steps[1]),
        [(
            call_ids[0],
            "later".into(),
            json!("external"),
            "running".into()
        )]
    );
    assert_eq!(steps[0]["tool_calls"][0]["input"], json!({"q": 1}));
}

#[sqlx::test]
async fn run_routes_404_and_403(pool: PgPool) {
    let id = seed_runs(&pool, 1).await[0];
    let client = client(&pool).await;

    let missing = client
        .get(format!("/runs/{}", id + 1000))
        .header(Header::new("Host", common::HOST))
        .dispatch()
        .await;
    assert_eq!(missing.status(), Status::NotFound);

    for path in ["/runs".to_string(), format!("/runs/{id}")] {
        for host in [Some("evil.example"), None] {
            let mut request = client.get(path.clone());
            if let Some(host) = host {
                request = request.header(Header::new("Host", host));
            }
            let response = request.dispatch().await;
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "{path} with Host {host:?}"
            );
        }
    }
}
