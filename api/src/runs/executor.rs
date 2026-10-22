//! Runs a workflow's steps for one `agent_run` job. Steps run in order, each one's output
//! feeding the next. Steps that already succeeded are skipped, so a retried, deferred,
//! requeued or resumed run picks up at its first unfinished step.
//!
//! Steps are at-least-once: a step that dies after its side effects but before its
//! `succeeded` write runs again, so step handlers (and the tools they call) must be
//! idempotent. Nothing enforces this.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::{RunStatus, StepStatus};
use crate::claude::{
    ClaudeError, ClaudeRequest, ClaudeResponse, ClaudeRunner, MAX_ERROR_BYTES, head,
};
use crate::events::EventBus;
use crate::jobs::Job;
use crate::tools::{Meter, Registry, Spend, ToolCallError};

/// A named, fixed sequence of steps.
#[rocket::async_trait]
pub trait Workflow: Send + Sync {
    fn name(&self) -> &'static str;
    /// The step names, in order. Stored on the run's steps when it starts.
    fn steps(&self) -> &'static [&'static str];
    /// Runs step `step` on `input`: the previous step's output, or the run's input for the
    /// first step. May run more than once for the same run, so it must be idempotent.
    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError>;
}

/// Why a step failed.
#[derive(Debug, Clone, PartialEq)]
pub enum StepError {
    /// Another attempt might work; the job retries while it has attempts left.
    Retry(String),
    /// The subscription's usage limit was hit; the job is deferred and the queue pauses.
    UsageLimit(String),
    /// Retrying won't help; the run fails.
    Fail(String),
}

impl StepError {
    fn message(&self) -> &str {
        match self {
            Self::Retry(message) | Self::UsageLimit(message) | Self::Fail(message) => message,
        }
    }
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// The one mapping from runner errors to queue outcomes: a usage limit defers, every other
/// runner error is worth a retry. The worker's other job kinds use it too.
impl From<ClaudeError> for StepError {
    fn from(err: ClaudeError) -> Self {
        match err {
            ClaudeError::UsageLimit { .. } => Self::UsageLimit(err.to_string()),
            _ => Self::Retry(err.to_string()),
        }
    }
}

impl From<ToolCallError> for StepError {
    fn from(err: ToolCallError) -> Self {
        match err {
            ToolCallError::UnknownTool
            | ToolCallError::InvalidInput(_)
            | ToolCallError::InvalidOutput(_) => Self::Fail(err.to_string()),
            ToolCallError::Failed(_) | ToolCallError::Database(_) => Self::Retry(err.to_string()),
            ToolCallError::Claude(err) => {
                let message = format!("the tool's model call failed: {err}");
                match Self::from(err) {
                    Self::UsageLimit(_) => Self::UsageLimit(message),
                    _ => Self::Retry(message),
                }
            }
        }
    }
}

/// What one `agent_run` job attempt decided; the worker maps it 1:1 to its job outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done(Value),
    Retry(String),
    UsageLimit(String),
    Fail(String),
}

impl From<StepError> for Outcome {
    fn from(err: StepError) -> Self {
        match err {
            StepError::Retry(message) => Self::Retry(message),
            StepError::UsageLimit(message) => Self::UsageLimit(message),
            StepError::Fail(message) => Self::Fail(message),
        }
    }
}

/// The workflows and tools the worker can run. The production runtime is empty for now.
#[derive(Clone, Default)]
pub struct Runtime {
    workflows: Arc<HashMap<&'static str, Arc<dyn Workflow>>>,
    tools: Arc<Registry>,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("workflows", &self.workflows.keys().collect::<Vec<_>>())
            .field("tools", &self.tools)
            .finish()
    }
}

impl Runtime {
    /// Refuses two workflows with one name, a workflow without steps and a workflow with two
    /// steps of one name.
    pub fn new(workflows: Vec<Arc<dyn Workflow>>, tools: Registry) -> Result<Self, String> {
        let mut by_name = HashMap::new();
        for workflow in workflows {
            let name = workflow.name();
            let steps = workflow.steps();
            if steps.is_empty() {
                return Err(format!("workflow {name:?} has no steps"));
            }
            let mut seen = HashSet::new();
            if let Some(step) = steps.iter().find(|step| !seen.insert(**step)) {
                return Err(format!("workflow {name:?} has two steps named {step:?}"));
            }
            if by_name.insert(name, workflow).is_some() {
                return Err(format!("two workflows are named {name:?}"));
            }
        }
        Ok(Self {
            workflows: Arc::new(by_name),
            tools: Arc::new(tools),
        })
    }

    pub fn workflow(&self, name: &str) -> Option<&Arc<dyn Workflow>> {
        self.workflows.get(name)
    }

    pub fn tools(&self) -> &Registry {
        &self.tools
    }
}

/// What a step's handler gets: the pool, metered model calls and the tool registry. Model
/// calls and tool calls made through it are added to the step's tokens and cost.
pub struct StepContext<'a> {
    pool: &'a PgPool,
    runner: &'a ClaudeRunner,
    tools: &'a Registry,
    bus: &'a EventBus,
    run_id: i64,
    step_id: i64,
    meter: Meter,
}

impl StepContext<'_> {
    pub fn pool(&self) -> &PgPool {
        self.pool
    }

    /// The `run_steps.id` this context belongs to.
    pub fn step_id(&self) -> i64 {
        self.step_id
    }

    /// [`ClaudeRunner::call`], counted against this step.
    pub async fn call(&mut self, req: &ClaudeRequest) -> Result<ClaudeResponse, ClaudeError> {
        self.meter.call(self.pool, self.runner, req).await
    }

    /// [`ClaudeRunner::call_structured`], counted against this step.
    pub async fn call_structured<T: DeserializeOwned>(
        &mut self,
        req: &ClaudeRequest,
        schema: &Value,
    ) -> Result<(T, ClaudeResponse), ClaudeError> {
        self.meter
            .call_structured(self.pool, self.runner, req, schema)
            .await
    }

    /// Invokes tool `name`, recording it in `tool_calls`. Its model calls count against
    /// this step even when it fails.
    pub async fn tool(&mut self, name: &str, input: Value) -> Result<Value, ToolCallError> {
        let (spend, result) = self
            .tools
            .invoke_metered(self.pool, self.runner, self.step_id, None, name, input)
            .await;
        self.meter.spend.add(spend);
        super::publish(self.pool, self.bus, self.run_id).await;
        result
    }
}

/// The parts of a run the executor reads.
#[derive(sqlx::FromRow)]
struct RunRow {
    workflow: String,
    status: RunStatus,
    input: Value,
}

#[derive(sqlx::FromRow)]
struct StepRow {
    id: i64,
    name: String,
    status: StepStatus,
    output: Option<Value>,
}

/// Runs the unfinished steps of the run in `job`'s payload and records the run's status
/// (see docs/agent-runtime.md for the table). A database error gives [`Outcome::Retry`].
pub async fn execute(
    pool: &PgPool,
    runner: &ClaudeRunner,
    runtime: &Runtime,
    bus: &EventBus,
    job: &Job,
) -> Outcome {
    let Some(run_id) = job.payload.get("run_id").and_then(Value::as_i64) else {
        return Outcome::Fail("invalid agent_run payload: run_id must be an integer".into());
    };
    match run(pool, runner, runtime, bus, job, run_id).await {
        Ok(outcome) => outcome,
        Err(err) => {
            let outcome = Outcome::Retry(format!("database error in run {run_id}: {err}"));
            // Best effort, so the run doesn't stay `running` once the job gives up.
            let settled = settle(pool, run_id, job, &outcome, Spend::default()).await;
            if let Err(err) = settled {
                rocket::warn!("runs: cannot record the status of run {run_id}: {err}");
            }
            super::publish(pool, bus, run_id).await;
            outcome
        }
    }
}

async fn run(
    pool: &PgPool,
    runner: &ClaudeRunner,
    runtime: &Runtime,
    bus: &EventBus,
    job: &Job,
    run_id: i64,
) -> Result<Outcome, sqlx::Error> {
    let found: Option<RunRow> =
        sqlx::query_as("SELECT workflow, status, input FROM agent_runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(pool)
            .await?;
    let Some(found) = found else {
        return Ok(Outcome::Fail(format!("run {run_id} does not exist")));
    };
    if found.status == RunStatus::Succeeded {
        return Ok(Outcome::Done(json!({"run_id": run_id})));
    }
    let Some(workflow) = runtime.workflow(&found.workflow) else {
        let outcome = Outcome::Fail(format!(
            "no workflow is named {:?} in this runtime",
            found.workflow
        ));
        settle(pool, run_id, job, &outcome, Spend::default()).await?;
        super::publish(pool, bus, run_id).await;
        return Ok(outcome);
    };

    sqlx::query(
        "UPDATE agent_runs SET status = 'running', started_at = COALESCE(started_at, now()), \
         updated_at = now() WHERE id = $1",
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    super::publish(pool, bus, run_id).await;
    let steps: Vec<StepRow> = sqlx::query_as(
        "SELECT id, name, status, output FROM run_steps WHERE run_id = $1 ORDER BY ordinal",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    let mut carried = found.input;
    for step in steps {
        if step.status == StepStatus::Succeeded {
            carried = step.output.unwrap_or(Value::Null);
            continue;
        }
        sqlx::query(
            "UPDATE run_steps SET status = 'running', attempts = attempts + 1, input = $2, \
             started_at = now(), finished_at = NULL, updated_at = now() WHERE id = $1",
        )
        .bind(step.id)
        .bind(&carried)
        .execute(pool)
        .await?;
        super::publish(pool, bus, run_id).await;

        let mut ctx = StepContext {
            pool,
            runner,
            tools: runtime.tools(),
            bus,
            run_id,
            step_id: step.id,
            meter: Meter::default(),
        };
        let result = workflow.run_step(&mut ctx, &step.name, carried).await;
        let Meter { spend, last_call } = ctx.meter;

        let mut tx = pool.begin().await?;
        match result {
            Ok(output) => {
                sqlx::query(
                    "UPDATE run_steps SET status = 'succeeded', output = $2, error = NULL, \
                     input_tokens = input_tokens + $3, output_tokens = output_tokens + $4, \
                     cost_usd = cost_usd + $5, claude_call_id = COALESCE($6, claude_call_id), \
                     finished_at = now(), updated_at = now() WHERE id = $1",
                )
                .bind(step.id)
                .bind(&output)
                .bind(spend.input_tokens)
                .bind(spend.output_tokens)
                .bind(spend.cost_usd)
                .bind(last_call)
                .execute(&mut *tx)
                .await?;
                add_run_spend(&mut tx, run_id, spend).await?;
                tx.commit().await?;
                super::publish(pool, bus, run_id).await;
                carried = output;
            }
            Err(err) => {
                let message = format!("step {:?}: {err}", step.name);
                sqlx::query(
                    "UPDATE run_steps SET status = 'failed', error = $2, \
                     input_tokens = input_tokens + $3, output_tokens = output_tokens + $4, \
                     cost_usd = cost_usd + $5, claude_call_id = COALESCE($6, claude_call_id), \
                     finished_at = now(), updated_at = now() WHERE id = $1",
                )
                .bind(step.id)
                .bind(head(&message, MAX_ERROR_BYTES))
                .bind(spend.input_tokens)
                .bind(spend.output_tokens)
                .bind(spend.cost_usd)
                .bind(last_call)
                .execute(&mut *tx)
                .await?;
                let outcome = Outcome::from(match err {
                    StepError::Retry(_) => StepError::Retry(message),
                    StepError::UsageLimit(_) => StepError::UsageLimit(message),
                    StepError::Fail(_) => StepError::Fail(message),
                });
                settle(&mut *tx, run_id, job, &outcome, spend).await?;
                tx.commit().await?;
                super::publish(pool, bus, run_id).await;
                return Ok(outcome);
            }
        }
    }

    sqlx::query(
        "UPDATE agent_runs SET status = 'succeeded', output = $2, error = NULL, \
         finished_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(run_id)
    .bind(&carried)
    .execute(pool)
    .await?;
    super::publish(pool, bus, run_id).await;
    Ok(Outcome::Done(json!({"run_id": run_id})))
}

async fn add_run_spend(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: i64,
    spend: Spend,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agent_runs SET input_tokens = input_tokens + $2, \
         output_tokens = output_tokens + $3, cost_usd = cost_usd + $4, updated_at = now() \
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(spend.input_tokens)
    .bind(spend.output_tokens)
    .bind(spend.cost_usd)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records a failed attempt on the run: `queued` while the job will run again (a retry
/// with attempts left, or a usage-limit deferral), otherwise `failed` with `finished_at`.
/// Adds `spend` to the run's totals.
async fn settle<'e>(
    db: impl sqlx::PgExecutor<'e>,
    run_id: i64,
    job: &Job,
    outcome: &Outcome,
    spend: Spend,
) -> Result<(), sqlx::Error> {
    let (status, error) = match outcome {
        Outcome::Done(_) => return Ok(()),
        Outcome::Retry(error) if job.attempts < job.max_attempts => ("queued", error),
        Outcome::UsageLimit(error) => ("queued", error),
        Outcome::Retry(error) | Outcome::Fail(error) => ("failed", error),
    };
    sqlx::query(
        "UPDATE agent_runs SET status = $2, error = $3, input_tokens = input_tokens + $4, \
         output_tokens = output_tokens + $5, cost_usd = cost_usd + $6, \
         finished_at = CASE WHEN $2 = 'failed' THEN now() ELSE finished_at END, \
         updated_at = now() WHERE id = $1",
    )
    .bind(run_id)
    .bind(status)
    .bind(head(error, MAX_ERROR_BYTES))
    .bind(spend.input_tokens)
    .bind(spend.output_tokens)
    .bind(spend.cost_usd)
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Steps(&'static str, &'static [&'static str]);

    #[rocket::async_trait]
    impl Workflow for Steps {
        fn name(&self) -> &'static str {
            self.0
        }
        fn steps(&self) -> &'static [&'static str] {
            self.1
        }
        async fn run_step(
            &self,
            _: &mut StepContext<'_>,
            _: &str,
            input: Value,
        ) -> Result<Value, StepError> {
            Ok(input)
        }
    }

    fn runtime(workflows: Vec<Steps>) -> Result<Runtime, String> {
        let workflows = workflows
            .into_iter()
            .map(|workflow| Arc::new(workflow) as Arc<dyn Workflow>)
            .collect();
        Runtime::new(workflows, Registry::default())
    }

    #[test]
    fn runtime_rejects_bad_workflows() {
        let ok = runtime(vec![Steps("a", &["one", "two"]), Steps("b", &["one"])]).unwrap();
        assert!(ok.workflow("a").is_some());
        assert!(ok.workflow("c").is_none());

        let cases = [
            (vec![Steps("a", &[])], "workflow \"a\" has no steps"),
            (
                vec![Steps("a", &["x", "y", "x"])],
                "workflow \"a\" has two steps named \"x\"",
            ),
            (
                vec![Steps("a", &["x"]), Steps("a", &["y"])],
                "two workflows are named \"a\"",
            ),
        ];
        for (workflows, expected) in cases {
            assert_eq!(runtime(workflows).unwrap_err(), expected);
        }
    }

    #[test]
    fn errors_map_to_outcomes() {
        let usage = ClaudeError::UsageLimit {
            message: "hit".into(),
        };
        let timeout = ClaudeError::Timeout {
            after: std::time::Duration::from_secs(1),
        };
        assert!(matches!(
            StepError::from(usage.clone()),
            StepError::UsageLimit(_)
        ));
        assert!(matches!(
            StepError::from(timeout.clone()),
            StepError::Retry(_)
        ));

        let cases = [
            (ToolCallError::UnknownTool, "fail"),
            (ToolCallError::InvalidInput("$".into()), "fail"),
            (ToolCallError::InvalidOutput("$".into()), "fail"),
            (ToolCallError::Failed("boom".into()), "retry"),
            (ToolCallError::Database(sqlx::Error::PoolClosed), "retry"),
            (ToolCallError::Claude(timeout), "retry"),
            (ToolCallError::Claude(usage), "usage_limit"),
        ];
        for (err, expected) in cases {
            let label = err.to_string();
            let kind = match Outcome::from(StepError::from(err)) {
                Outcome::Retry(_) => "retry",
                Outcome::UsageLimit(_) => "usage_limit",
                Outcome::Fail(_) => "fail",
                Outcome::Done(_) => "done",
            };
            assert_eq!(kind, expected, "{label}");
        }
    }
}
