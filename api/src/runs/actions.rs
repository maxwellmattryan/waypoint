//! The action log: one `agent_actions` row per succeeded `write` tool call, written with the
//! call's final `tool_calls` update (see [`crate::tools`]), and undo. Undo reverts exactly
//! one action through its tool's [`crate::tools::Tool::undo`], and refuses rather than
//! overwrites a record that changed since.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::Json;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use super::Runtime;
use super::approvals::DecisionError;
use crate::applications::{ApiError, internal};
use crate::host::{LocalHost, NotCrossSite};
use crate::tools::{Registry, UndoError};

/// One `agent_actions` row: a `write` tool call that changed Waypoint's data.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AgentAction {
    pub id: i64,
    pub run_id: i64,
    pub run_step_id: i64,
    pub tool_call_id: i64,
    pub tool: String,
    /// What the tool's undo needs; `null` when the handler set none, so it can't be undone.
    #[schema(required = true)]
    pub undo_data: Option<Value>,
    pub created_at: DateTime<Utc>,
    /// When the action was undone; `null` while it stands.
    #[schema(required = true)]
    pub undone_at: Option<DateTime<Utc>>,
}

const ACTION_COLUMNS: &str =
    "id, run_id, run_step_id, tool_call_id, tool, undo_data, created_at, undone_at";

/// Up to `limit` actions with `id < before` (any id when `None`), of run `run_id` when
/// given, newest first.
pub async fn list_actions(
    pool: &PgPool,
    run_id: Option<i64>,
    limit: i64,
    before: Option<i64>,
) -> Result<Vec<AgentAction>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {ACTION_COLUMNS} FROM agent_actions \
         WHERE ($1::bigint IS NULL OR run_id = $1) AND ($3::bigint IS NULL OR id < $3) \
         ORDER BY id DESC LIMIT $2"
    ))
    .bind(run_id)
    .bind(limit)
    .bind(before)
    .fetch_all(pool)
    .await
}

/// Undoes action `id` in one transaction: locks it, runs its tool's undo on the same
/// connection and sets `undone_at`. Nothing changes when it is already undone, has no undo
/// data, its tool isn't registered or the undo reports a conflict.
pub async fn undo(
    pool: &PgPool,
    registry: &Registry,
    id: i64,
) -> Result<AgentAction, DecisionError> {
    let mut tx = pool.begin().await?;
    let action: AgentAction = sqlx::query_as(&format!(
        "SELECT {ACTION_COLUMNS} FROM agent_actions WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DecisionError::NotFound)?;
    if action.undone_at.is_some() {
        return Err(DecisionError::Conflict(format!(
            "action {id} is already undone"
        )));
    }
    let Some(data) = action.undo_data else {
        return Err(DecisionError::Conflict(format!(
            "action {id} has no undo data"
        )));
    };
    if registry.permission(&action.tool).is_none() {
        return Err(DecisionError::Conflict(format!(
            "tool {:?} is no longer registered",
            action.tool
        )));
    }
    match registry.undo(&action.tool, &mut tx, data).await {
        Ok(()) => {}
        Err(UndoError::Database(err)) => return Err(DecisionError::Db(err)),
        Err(err) => return Err(DecisionError::Conflict(format!("action {id}: {err}"))),
    }
    let action: AgentAction = sqlx::query_as(&format!(
        "UPDATE agent_actions SET undone_at = now() WHERE id = $1 RETURNING {ACTION_COLUMNS}"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(action)
}

/// `GET /actions`: the action log, newest first, a page at a time.
#[utoipa::path(
    get,
    path = "/actions",
    tag = "runs",
    operation_id = "list_actions",
    params(
        ("run_id" = Option<i64>, Query, description = "Only this run's actions"),
        ("limit" = Option<i64>, Query, description = "Most actions to return; default 50, clamped to 1..=200"),
        ("before" = Option<i64>, Query, description = "Only actions with an `id` below this one, for the next page"),
    ),
    responses(
        (status = 200, description = "Actions, `id` descending", body = [AgentAction]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The actions could not be read"),
    )
)]
#[rocket::get("/actions?<run_id>&<limit>&<before>")]
pub async fn list(
    _host: LocalHost,
    run_id: Option<i64>,
    limit: Option<i64>,
    before: Option<i64>,
    pool: &State<PgPool>,
) -> Result<Json<Vec<AgentAction>>, ApiError> {
    list_actions(pool, run_id, super::page_size(limit), before)
        .await
        .map(Json)
        .map_err(internal("list actions"))
}

/// `POST /actions/{id}/undo`: reverts one action.
#[utoipa::path(
    post,
    path = "/actions/{id}/undo",
    tag = "runs",
    operation_id = "undo_action",
    params(("id" = i64, Path, description = "The action's id")),
    responses(
        (status = 200, description = "The undone action, with `undone_at` set", body = AgentAction),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No action has this id"),
        (status = 409, description = "The action is already undone, has no undo data, its tool isn't registered, or the record changed or is gone"),
        (status = 500, description = "The undo could not be stored"),
    )
)]
#[rocket::post("/actions/<id>/undo")]
pub async fn undo_route(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    pool: &State<PgPool>,
    runtime: &State<Runtime>,
) -> Result<Json<AgentAction>, ApiError> {
    undo(pool, runtime.tools(), id)
        .await
        .map(Json)
        .map_err(|err| err.into_api("undo an action"))
}
