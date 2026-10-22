//! Agent runs: one `agent_runs` row per run of a workflow, one `run_steps` row per step and
//! one `tool_calls` row per tool invocation. A run is executed by one `agent_run` queue job
//! and resumes from its first unfinished step (see [`executor`] and docs/agent-runtime.md).
//! The routes here are read-only.

pub mod executor;

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;

pub use self::executor::{Outcome, Runtime, StepContext, StepError, Workflow, execute};
use crate::applications::{ApiError, internal};
use crate::events::EventBus;
use crate::host::LocalHost;
use crate::jobs::{self, Wake};
use crate::tools::Permission;

/// Runs listed when `GET /runs` has no `limit`.
pub const DEFAULT_LIMIT: i64 = 50;
/// Most runs one `GET /runs` returns.
pub const MAX_LIMIT: i64 = 200;

/// Where a run is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum RunStatus {
    /// Waiting for its job to run, including between retries and after a usage limit.
    Queued,
    Running,
    Succeeded,
    /// Stopped for good; `runs::resume` can queue it again.
    Failed,
}

/// Where a step is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum StepStatus {
    /// Not started yet.
    Pending,
    Running,
    Succeeded,
    /// The last attempt failed; the next run of the job retries it.
    Failed,
}

/// How a tool call ended.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ToolCallStatus {
    Running,
    Succeeded,
    /// The handler or one of its model calls failed.
    Failed,
    /// The input broke the tool's input schema; the handler didn't run.
    InvalidInput,
    /// The output broke the tool's output schema; it is stored anyway.
    InvalidOutput,
    /// No tool has the name; the handler didn't run.
    UnknownTool,
}

// The `Option` fields below are always serialized (as `null` when empty), so they are
// required in the schema.

/// One `agent_runs` row.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Run {
    pub id: i64,
    /// The registered workflow's name.
    pub workflow: String,
    pub status: RunStatus,
    /// The JSON the run was started with; the first step's input.
    pub input: Value,
    /// The last step's output, once the run has succeeded.
    #[schema(required = true)]
    pub output: Option<Value>,
    /// The latest step error; cleared when the run succeeds.
    #[schema(required = true)]
    pub error: Option<String>,
    /// Sum of the steps' input tokens (including cache tokens).
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Sum of the steps' reported cost in US dollars.
    pub cost_usd: f64,
    /// The latest `agent_run` job.
    #[schema(required = true)]
    pub job_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[schema(required = true)]
    pub started_at: Option<DateTime<Utc>>,
    #[schema(required = true)]
    pub finished_at: Option<DateTime<Utc>>,
}

/// One `run_steps` row.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RunStep {
    pub id: i64,
    pub run_id: i64,
    /// The step's position in the workflow, from 0.
    pub ordinal: i32,
    pub name: String,
    pub status: StepStatus,
    /// How many times the step has started.
    pub attempts: i32,
    /// The previous step's output (the run's input for the first step), once started.
    #[schema(required = true)]
    pub input: Option<Value>,
    #[schema(required = true)]
    pub output: Option<Value>,
    #[schema(required = true)]
    pub error: Option<String>,
    /// Tokens and cost of the step's model and tool calls, across attempts.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    /// The step's most recent successful model call.
    #[schema(required = true)]
    pub claude_call_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[schema(required = true)]
    pub started_at: Option<DateTime<Utc>>,
    #[schema(required = true)]
    pub finished_at: Option<DateTime<Utc>>,
}

/// One `tool_calls` row.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ToolCall {
    pub id: i64,
    pub run_step_id: i64,
    /// The tool's name as the caller gave it.
    pub tool: String,
    /// The tool's permission level; `null` for an unknown tool.
    #[schema(required = true)]
    pub permission: Option<Permission>,
    pub status: ToolCallStatus,
    pub input: Value,
    #[schema(required = true)]
    pub output: Option<Value>,
    #[schema(required = true)]
    pub error: Option<String>,
    /// Tokens and cost of the model calls the tool made.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    /// The model call that asked for the tool, if any.
    #[schema(required = true)]
    pub claude_call_id: Option<i64>,
    #[schema(required = true)]
    pub duration_ms: Option<i64>,
    pub created_at: DateTime<Utc>,
    #[schema(required = true)]
    pub finished_at: Option<DateTime<Utc>>,
}

/// A step with its tool calls, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct RunStepDetail {
    #[serde(flatten)]
    pub step: RunStep,
    pub tool_calls: Vec<ToolCall>,
}

/// The `GET /runs/{id}` body: the run with its steps in order.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct RunDetail {
    #[serde(flatten)]
    pub run: Run,
    pub steps: Vec<RunStepDetail>,
}

const RUN_COLUMNS: &str = "id, workflow, status, input, output, error, input_tokens, \
                           output_tokens, cost_usd, job_id, created_at, updated_at, \
                           started_at, finished_at";
const STEP_COLUMNS: &str = "id, run_id, ordinal, name, status, attempts, input, output, error, \
                            input_tokens, output_tokens, cost_usd, claude_call_id, created_at, \
                            updated_at, started_at, finished_at";
const TOOL_CALL_COLUMNS: &str = "id, run_step_id, tool, permission, status, input, output, \
                                 error, input_tokens, output_tokens, cost_usd, claude_call_id, \
                                 duration_ms, created_at, finished_at";

/// Why a run could not be started.
#[derive(Debug)]
pub enum StartError {
    /// The runtime has no workflow with this name.
    UnknownWorkflow(String),
    Db(sqlx::Error),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWorkflow(name) => write!(f, "no workflow is named {name:?}"),
            Self::Db(err) => write!(f, "cannot store the run: {err}"),
        }
    }
}

impl std::error::Error for StartError {}

impl From<sqlx::Error> for StartError {
    fn from(err: sqlx::Error) -> Self {
        Self::Db(err)
    }
}

/// Starts a run of `workflow` with `input`: stores the run (`queued`), one `pending` step
/// per workflow step and its `agent_run` job in one transaction, then wakes the worker.
pub async fn start(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    runtime: &Runtime,
    workflow: &str,
    input: Value,
) -> Result<Run, StartError> {
    let Some(found) = runtime.workflow(workflow) else {
        return Err(StartError::UnknownWorkflow(workflow.to_owned()));
    };
    let steps: Vec<&str> = found.steps().to_vec();

    let mut tx = pool.begin().await?;
    let run_id: i64 =
        sqlx::query_scalar("INSERT INTO agent_runs (workflow, input) VALUES ($1, $2) RETURNING id")
            .bind(found.name())
            .bind(&input)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT INTO run_steps (run_id, ordinal, name) \
         SELECT $1, (step.ordinal - 1)::int, step.name \
         FROM unnest($2::text[]) WITH ORDINALITY AS step (name, ordinal)",
    )
    .bind(run_id)
    .bind(&steps)
    .execute(&mut *tx)
    .await?;
    let run = enqueue(&mut tx, bus, run_id).await?;
    tx.commit().await?;
    wake.0.notify_one();
    Ok(run)
}

/// Queues a `failed` run again; its job skips the steps that already succeeded. Returns
/// `None`, and queues nothing, for any other status or an unknown id, so a run never has
/// two jobs in flight.
pub async fn resume(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    run_id: i64,
) -> Result<Option<Run>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let reopened: Option<i64> = sqlx::query_scalar(
        "UPDATE agent_runs SET status = 'queued', finished_at = NULL, updated_at = now() \
         WHERE id = $1 AND status = 'failed' RETURNING id",
    )
    .bind(run_id)
    .fetch_optional(&mut *tx)
    .await?;
    if reopened.is_none() {
        return Ok(None);
    }
    let run = enqueue(&mut tx, bus, run_id).await?;
    tx.commit().await?;
    wake.0.notify_one();
    Ok(Some(run))
}

/// Inserts the run's `agent_run` job and points the run at it.
async fn enqueue(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bus: &EventBus,
    run_id: i64,
) -> Result<Run, sqlx::Error> {
    let job = jobs::enqueue_tx(
        tx,
        bus,
        jobs::AGENT_RUN,
        json!({"run_id": run_id}),
        jobs::Config::default().max_attempts,
    )
    .await?;
    sqlx::query_as(&format!(
        "UPDATE agent_runs SET job_id = $2, updated_at = now() WHERE id = $1 \
         RETURNING {RUN_COLUMNS}"
    ))
    .bind(run_id)
    .bind(job.id)
    .fetch_one(&mut **tx)
    .await
}

/// `limit` for `GET /runs`: [`DEFAULT_LIMIT`] when absent, clamped to `1..=MAX_LIMIT`.
pub fn page_size(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Up to `limit` runs with `id < before` (any id when `None`), newest first.
pub async fn list_runs(
    pool: &PgPool,
    limit: i64,
    before: Option<i64>,
) -> Result<Vec<Run>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE $2::bigint IS NULL OR id < $2 \
         ORDER BY id DESC LIMIT $1"
    ))
    .bind(limit)
    .bind(before)
    .fetch_all(pool)
    .await
}

/// A run with its steps by ordinal and each step's tool calls by id.
pub async fn get_detail(pool: &PgPool, id: i64) -> Result<Option<RunDetail>, sqlx::Error> {
    let Some(run) = sqlx::query_as::<_, Run>(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let steps: Vec<RunStep> = sqlx::query_as(&format!(
        "SELECT {STEP_COLUMNS} FROM run_steps WHERE run_id = $1 ORDER BY ordinal"
    ))
    .bind(id)
    .fetch_all(pool)
    .await?;
    let step_ids: Vec<i64> = steps.iter().map(|step| step.id).collect();
    let calls: Vec<ToolCall> = sqlx::query_as(&format!(
        "SELECT {TOOL_CALL_COLUMNS} FROM tool_calls WHERE run_step_id = ANY($1) \
         ORDER BY run_step_id, id"
    ))
    .bind(&step_ids)
    .fetch_all(pool)
    .await?;

    let mut by_step: HashMap<i64, Vec<ToolCall>> = HashMap::new();
    for call in calls {
        by_step.entry(call.run_step_id).or_default().push(call);
    }
    let steps = steps
        .into_iter()
        .map(|step| RunStepDetail {
            tool_calls: by_step.remove(&step.id).unwrap_or_default(),
            step,
        })
        .collect();
    Ok(Some(RunDetail { run, steps }))
}

/// `GET /runs`: runs newest first, a page at a time.
#[utoipa::path(
    get,
    path = "/runs",
    tag = "runs",
    operation_id = "list_runs",
    params(
        ("limit" = Option<i64>, Query, description = "Most runs to return; default 50, clamped to 1..=200"),
        ("before" = Option<i64>, Query, description = "Only runs with an `id` below this one, for the next page"),
    ),
    responses(
        (status = 200, description = "Runs, `id` descending", body = [Run]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The runs could not be read"),
    )
)]
#[rocket::get("/runs?<limit>&<before>")]
pub async fn list(
    _host: LocalHost,
    limit: Option<i64>,
    before: Option<i64>,
    pool: &State<PgPool>,
) -> Result<Json<Vec<Run>>, ApiError> {
    list_runs(pool, page_size(limit), before)
        .await
        .map(Json)
        .map_err(internal("list runs"))
}

/// `GET /runs/{id}`: one run with its steps and their tool calls.
#[utoipa::path(
    get,
    path = "/runs/{id}",
    tag = "runs",
    operation_id = "get_run",
    params(("id" = i64, Path, description = "The run's id")),
    responses(
        (status = 200, description = "The run, its steps by `ordinal` and each step's tool calls by `id`", body = RunDetail),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No run has this id"),
        (status = 500, description = "The run could not be read"),
    )
)]
#[rocket::get("/runs/<id>")]
pub async fn show(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<RunDetail>, ApiError> {
    get_detail(pool, id)
        .await
        .map_err(internal("read a run"))?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_size_defaults_and_clamps() {
        let cases = [
            (None, 50),
            (Some(0), 1),
            (Some(-5), 1),
            (Some(1), 1),
            (Some(200), 200),
            (Some(201), 200),
            (Some(i64::MAX), 200),
        ];
        for (limit, expected) in cases {
            assert_eq!(page_size(limit), expected, "{limit:?}");
        }
    }
}
