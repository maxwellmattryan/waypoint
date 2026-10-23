//! The approval gate's decisions. An `external` tool call creates a `pending_actions` row
//! and parks its step (see [`crate::tools`] and [`super::executor`]); here the user lists
//! those actions and approves (optionally with an edited input) or rejects them.
//!
//! Both decisions lock the pending action, then its run, so they serialize with each other
//! and with the executor parking the run.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use super::{Run, RunStatus, Runtime};
use crate::applications::{ApiError, FieldError, JsonContent, ValidationErrors, internal, parsed};
use crate::events::{Event, EventBus};
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::Wake;
use crate::tools::{Registry, ToolCallError};

/// Most pending actions one `GET /pending-actions` returns.
pub const MAX_PENDING: i64 = 200;

/// Where a pending action is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum PendingActionStatus {
    /// Waiting for the user's decision.
    Pending,
    /// Approved; the handler runs when the step runs again.
    Approved,
    /// The user rejected it; the handler never runs.
    Rejected,
    /// The handler ran with `approved_input`; `output` is what it returned.
    Executed,
}

/// One `pending_actions` row: an `external` tool call waiting for, or past, the user's
/// decision.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct PendingAction {
    pub id: i64,
    pub run_id: i64,
    pub run_step_id: i64,
    /// The tool call that asked for the action.
    pub tool_call_id: i64,
    pub tool: String,
    /// The input the caller asked for.
    pub input: Value,
    /// The input the user approved: the edited input, or a copy of `input`.
    #[schema(required = true)]
    pub approved_input: Option<Value>,
    pub status: PendingActionStatus,
    /// The handler's output, once executed.
    #[schema(required = true)]
    pub output: Option<Value>,
    /// The tool call that ran the handler, once executed.
    #[schema(required = true)]
    pub executed_tool_call_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    #[schema(required = true)]
    pub decided_at: Option<DateTime<Utc>>,
    #[schema(required = true)]
    pub executed_at: Option<DateTime<Utc>>,
}

pub(crate) const PENDING_ACTION_COLUMNS: &str = "id, run_id, run_step_id, tool_call_id, tool, \
                                                 input, approved_input, status, output, \
                                                 executed_tool_call_id, created_at, \
                                                 decided_at, executed_at";

/// The `POST /pending-actions/{id}/approve` body: `{}` approves the requested input,
/// `{"input": {…}}` approves an edited one.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveAction {
    /// Replaces the requested input; checked against the tool's input schema.
    #[serde(default)]
    pub input: Option<Value>,
}

/// Why a decision or an undo changed nothing.
#[derive(Debug)]
pub enum DecisionError {
    /// No row has this id.
    NotFound,
    /// The row or its run is not in a state that allows it; the reason is logged.
    Conflict(String),
    /// The edited input is invalid; the message names the path and never quotes values.
    Invalid(String),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for DecisionError {
    fn from(err: sqlx::Error) -> Self {
        Self::Db(err)
    }
}

impl DecisionError {
    /// The API answer, logging what the client doesn't see: a 409's reason and a database
    /// error (as `what`).
    pub(crate) fn into_api(self, what: &str) -> ApiError {
        match self {
            Self::NotFound => ApiError::NotFound,
            Self::Conflict(reason) => {
                rocket::info!("runs: cannot {what}: {reason}");
                ApiError::Conflict
            }
            Self::Invalid(message) => ApiError::Invalid(ValidationErrors {
                errors: vec![FieldError {
                    field: "input".into(),
                    message,
                }],
            }),
            Self::Db(err) => internal(what)(err),
        }
    }
}

/// Up to [`MAX_PENDING`] `pending` actions, oldest first.
pub async fn list_pending(pool: &PgPool) -> Result<Vec<PendingAction>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {PENDING_ACTION_COLUMNS} FROM pending_actions WHERE status = 'pending' \
         ORDER BY id LIMIT $1"
    ))
    .bind(MAX_PENDING)
    .fetch_all(pool)
    .await
}

/// Locks pending action `id` and its run, and checks that the action is `pending` and the
/// run `awaiting_approval`.
async fn lock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<PendingAction, DecisionError> {
    let action: PendingAction = sqlx::query_as(&format!(
        "SELECT {PENDING_ACTION_COLUMNS} FROM pending_actions WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(DecisionError::NotFound)?;
    let run_status: RunStatus =
        sqlx::query_scalar("SELECT status FROM agent_runs WHERE id = $1 FOR UPDATE")
            .bind(action.run_id)
            .fetch_one(&mut **tx)
            .await?;
    if action.status != PendingActionStatus::Pending {
        return Err(DecisionError::Conflict(format!(
            "pending action {id} is {:?}, not pending",
            action.status
        )));
    }
    if run_status != RunStatus::AwaitingApproval {
        return Err(DecisionError::Conflict(format!(
            "run {} is {run_status:?}, not awaiting approval",
            action.run_id
        )));
    }
    Ok(action)
}

/// Approves pending action `id`, with `input` replacing the requested input when given.
/// When it was the run's last `pending` action, the run is queued again in the same
/// transaction, the worker is woken and `run.updated` is published after commit.
pub async fn approve(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    registry: &Registry,
    id: i64,
    input: Option<Value>,
) -> Result<PendingAction, DecisionError> {
    let mut tx = pool.begin().await?;
    let action = lock(&mut tx, id).await?;
    if registry.permission(&action.tool).is_none() {
        return Err(DecisionError::Conflict(format!(
            "tool {:?} is no longer registered",
            action.tool
        )));
    }
    let approved = match input {
        Some(edited) => {
            registry
                .check_input(&action.tool, &edited)
                .map_err(|err| match err {
                    ToolCallError::InvalidInput(message) => DecisionError::Invalid(message),
                    other => DecisionError::Conflict(other.to_string()),
                })?;
            edited
        }
        None => action.input.clone(),
    };
    let action: PendingAction = sqlx::query_as(&format!(
        "UPDATE pending_actions SET status = 'approved', approved_input = $2, \
         decided_at = now() WHERE id = $1 RETURNING {PENDING_ACTION_COLUMNS}"
    ))
    .bind(id)
    .bind(&approved)
    .fetch_one(&mut *tx)
    .await?;
    let others: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pending_actions WHERE run_id = $1 AND status = 'pending'",
    )
    .bind(action.run_id)
    .fetch_one(&mut *tx)
    .await?;
    let requeued: Option<Run> = if others == 0 {
        sqlx::query("UPDATE agent_runs SET status = 'queued', updated_at = now() WHERE id = $1")
            .bind(action.run_id)
            .execute(&mut *tx)
            .await?;
        Some(super::enqueue(&mut tx, bus, action.run_id).await?)
    } else {
        None
    };
    tx.commit().await?;
    if let Some(run) = requeued {
        bus.publish(Event::RunUpdated { run });
        wake.0.notify_one();
    }
    Ok(action)
}

/// Rejects pending action `id` and every other `pending` action of its run, fails the run's
/// parked steps and ends the run `rejected`, in one transaction. The handler never runs.
/// Publishes `run.updated` after commit.
pub async fn reject(
    pool: &PgPool,
    bus: &EventBus,
    id: i64,
) -> Result<PendingAction, DecisionError> {
    let mut tx = pool.begin().await?;
    let action = lock(&mut tx, id).await?;
    let error = format!("the user rejected pending action {id}");
    sqlx::query(
        "UPDATE pending_actions SET status = 'rejected', decided_at = now() \
         WHERE run_id = $1 AND status = 'pending'",
    )
    .bind(action.run_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE run_steps SET status = 'failed', error = $2, finished_at = now(), \
         updated_at = now() WHERE run_id = $1 AND status = 'awaiting_approval'",
    )
    .bind(action.run_id)
    .bind(&error)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE agent_runs SET status = 'rejected', error = $2, finished_at = now(), \
         updated_at = now() WHERE id = $1",
    )
    .bind(action.run_id)
    .bind(&error)
    .execute(&mut *tx)
    .await?;
    let action: PendingAction = sqlx::query_as(&format!(
        "SELECT {PENDING_ACTION_COLUMNS} FROM pending_actions WHERE id = $1"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    super::publish(pool, bus, action.run_id).await;
    Ok(action)
}

/// `GET /pending-actions`: the actions waiting for the user's decision.
#[utoipa::path(
    get,
    path = "/pending-actions",
    tag = "runs",
    operation_id = "list_pending_actions",
    responses(
        (status = 200, description = "Up to 200 `pending` actions, `id` ascending (oldest first)", body = [PendingAction]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The pending actions could not be read"),
    )
)]
#[rocket::get("/pending-actions")]
pub async fn list(
    _host: LocalHost,
    pool: &State<PgPool>,
) -> Result<Json<Vec<PendingAction>>, ApiError> {
    list_pending(pool)
        .await
        .map(Json)
        .map_err(internal("list pending actions"))
}

/// `POST /pending-actions/{id}/approve`: approves an action, optionally with an edited
/// input. Approving the run's last pending action queues the run again.
#[utoipa::path(
    post,
    path = "/pending-actions/{id}/approve",
    tag = "runs",
    operation_id = "approve_pending_action",
    params(("id" = i64, Path, description = "The pending action's id")),
    request_body = ApproveAction,
    responses(
        (status = 200, description = "The approved action", body = PendingAction),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No pending action has this id"),
        (status = 409, description = "The action is not `pending`, its run is not `awaiting_approval`, or its tool is no longer registered"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "The edited `input` breaks the tool's input schema", body = ValidationErrors),
        (status = 500, description = "The decision could not be stored"),
    )
)]
#[rocket::post("/pending-actions/<id>/approve", data = "<body>")]
#[allow(clippy::too_many_arguments)]
pub async fn approve_route(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    runtime: &State<Runtime>,
    body: Result<Json<ApproveAction>, json::Error<'_>>,
) -> Result<Json<PendingAction>, ApiError> {
    let ApproveAction { input } = parsed(body)?;
    approve(pool, bus, wake, runtime.tools(), id, input)
        .await
        .map(Json)
        .map_err(|err| err.into_api("approve a pending action"))
}

/// `POST /pending-actions/{id}/reject`: rejects an action, which ends its run `rejected`.
#[utoipa::path(
    post,
    path = "/pending-actions/{id}/reject",
    tag = "runs",
    operation_id = "reject_pending_action",
    params(("id" = i64, Path, description = "The pending action's id")),
    responses(
        (status = 200, description = "The rejected action", body = PendingAction),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No pending action has this id"),
        (status = 409, description = "The action is not `pending` or its run is not `awaiting_approval`"),
        (status = 500, description = "The decision could not be stored"),
    )
)]
#[rocket::post("/pending-actions/<id>/reject")]
pub async fn reject_route(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
) -> Result<Json<PendingAction>, ApiError> {
    reject(pool, bus, id)
        .await
        .map(Json)
        .map_err(|err| err.into_api("reject a pending action"))
}
