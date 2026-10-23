//! The kill switch and delete-all (see docs/agent-runtime.md and docs/google-oauth.md).
//!
//! The kill switch is one `kill_switch` row. While it is on, no agent run starts a step:
//! `queued` runs become `paused`, a `running` run stops before its next step, the worker
//! claims no `agent_run` job, and a started or resumed run is stored `paused`. Turning it
//! off makes every `paused` run `queued` again.
//!
//! Routes: `GET /kill-switch`, `PUT /kill-switch` and `DELETE /data`, which pauses agents,
//! revokes the Google grant and empties every user table.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, PgPool};

use crate::applications::{ApiError, FieldError, ValidationErrors, internal, parsed};
use crate::events::{Event, EventBus};
use crate::google::{self, Google, GoogleRevocation};
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::Wake;
use crate::runs::{RUN_COLUMNS, Run};

/// The phrase `DELETE /data` must be sent, exactly.
pub const CONFIRMATION: &str = "delete all data";
/// What a wrong phrase gets.
const CONFIRMATION_MESSAGE: &str = "type \"delete all data\" to confirm";

/// Every table `DELETE /data` empties: all of them except `_sqlx_migrations` and
/// `kill_switch`. A new table that references one of these makes the `TRUNCATE` fail
/// until it is added here.
pub const USER_TABLES: &[&str] = &[
    "plan_shortlist",
    "plan_step_tasks",
    "plan_steps",
    "plans",
    "goal_questions",
    "goals",
    "applications",
    "application_stages",
    "application_events",
    "resume_versions",
    "memories",
    "memory_signals",
    "memory_evidence",
    "agent_runs",
    "run_steps",
    "tool_calls",
    "pending_actions",
    "agent_actions",
    "application_packets",
    "posting_fit_scores",
    "review_items",
    "run_memories",
    "interview_events",
    "email_matches",
    "email_classifications",
    "emails",
    "email_threads",
    "gmail_sync_state",
    "claude_calls",
    "job_postings",
    "jobs",
    "google_accounts",
    "google_oauth_states",
];

/// The kill switch: the `GET /kill-switch` body.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct KillSwitch {
    /// Whether agents are paused.
    pub paused: bool,
    /// When `paused` last changed.
    pub updated_at: DateTime<Utc>,
}

/// The `PUT /kill-switch` body.
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetKillSwitch {
    pub paused: bool,
}

/// The `DELETE /data` body.
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteAllData {
    /// Must be exactly `delete all data`.
    pub confirmation: String,
}

/// The `DELETE /data` body on success.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct DataDeletion {
    pub google: GoogleRevocation,
}

pub async fn get<'e>(db: impl PgExecutor<'e>) -> Result<KillSwitch, sqlx::Error> {
    sqlx::query_as("SELECT paused, updated_at FROM kill_switch WHERE id")
        .fetch_one(db)
        .await
}

pub async fn is_paused<'e>(db: impl PgExecutor<'e>) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT paused FROM kill_switch WHERE id")
        .fetch_one(db)
        .await
}

/// Turns the kill switch on or off. On a change, moves every `queued` run to `paused`
/// (or every `paused` run back to `queued`) in the same transaction, then publishes
/// `run.updated` per moved run and `kill_switch.updated`, and on unpause wakes the worker.
/// Setting the current value changes and publishes nothing.
pub async fn set_paused(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    paused: bool,
) -> Result<KillSwitch, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let changed: Option<KillSwitch> = sqlx::query_as(
        "UPDATE kill_switch SET paused = $1, updated_at = now() WHERE id AND paused <> $1 \
         RETURNING paused, updated_at",
    )
    .bind(paused)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(state) = changed else {
        let current = get(&mut *tx).await?;
        tx.commit().await?;
        return Ok(current);
    };
    let (from, to) = if paused {
        ("queued", "paused")
    } else {
        ("paused", "queued")
    };
    let moved: Vec<Run> = sqlx::query_as(&format!(
        "UPDATE agent_runs SET status = $2, updated_at = now() WHERE status = $1 \
         RETURNING {RUN_COLUMNS}"
    ))
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    for run in moved {
        bus.publish(Event::RunUpdated { run });
    }
    bus.publish(Event::KillSwitchUpdated {
        kill_switch: state.clone(),
    });
    if !paused {
        wake.0.notify_one();
    }
    Ok(state)
}

/// `GET /kill-switch`: whether agents are paused.
#[utoipa::path(
    get,
    path = "/kill-switch",
    tag = "kill_switch",
    operation_id = "get_kill_switch",
    responses(
        (status = 200, description = "The kill switch", body = KillSwitch),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The kill switch could not be read"),
    )
)]
#[rocket::get("/kill-switch")]
pub async fn show(_host: LocalHost, pool: &State<PgPool>) -> Result<Json<KillSwitch>, ApiError> {
    get(pool.inner())
        .await
        .map(Json)
        .map_err(internal("read the kill switch"))
}

/// `PUT /kill-switch`: pauses or resumes every agent run. Setting the current value is a
/// no-op that returns the current state.
#[utoipa::path(
    put,
    path = "/kill-switch",
    tag = "kill_switch",
    operation_id = "set_kill_switch",
    request_body = SetKillSwitch,
    responses(
        (status = 200, description = "The kill switch after the change", body = KillSwitch),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 500, description = "The kill switch could not be changed"),
    )
)]
#[rocket::put("/kill-switch", data = "<body>")]
pub async fn update(
    _host: LocalHost,
    _site: NotCrossSite,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<SetKillSwitch>, json::Error<'_>>,
) -> Result<Json<KillSwitch>, ApiError> {
    let SetKillSwitch { paused } = parsed(body)?;
    set_paused(pool, bus, wake, paused)
        .await
        .map(Json)
        .map_err(internal("set the kill switch"))
}

/// `DELETE /data`: pauses agents (they stay paused), revokes the Google grant
/// (best-effort) and empties every user table.
#[utoipa::path(
    delete,
    path = "/data",
    tag = "kill_switch",
    operation_id = "delete_all_data",
    request_body = DeleteAllData,
    responses(
        (status = 200, description = "Every user table is empty and agents are paused; `google` says what happened to the Google grant", body = DataDeletion),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 422, description = "`confirmation` is not exactly `delete all data`", body = ValidationErrors),
        (status = 500, description = "The data could not be deleted; nothing was deleted"),
    )
)]
#[rocket::delete("/data", data = "<body>")]
pub async fn delete_all(
    _host: LocalHost,
    _site: NotCrossSite,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    google: &State<Google>,
    body: Result<Json<DeleteAllData>, json::Error<'_>>,
) -> Result<Json<DataDeletion>, ApiError> {
    let DeleteAllData { confirmation } = parsed(body)?;
    if confirmation != CONFIRMATION {
        return Err(ApiError::Invalid(ValidationErrors {
            errors: vec![FieldError {
                field: "confirmation".into(),
                message: CONFIRMATION_MESSAGE.into(),
            }],
        }));
    }

    delete_all_data(pool, bus, wake, google)
        .await
        .map(|google| Json(DataDeletion { google }))
        .map_err(internal("delete all data"))
}

/// What `DELETE /data` does once the phrase matches, in order: pauses agents (so no step
/// starts during the wipe), revokes the Google grant best-effort while its row still
/// exists, empties [`USER_TABLES`] in one statement, then publishes `data.deleted`. A
/// database error before the `TRUNCATE` leaves the data untouched (the pause may stay).
pub async fn delete_all_data(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    google: &Google,
) -> Result<GoogleRevocation, sqlx::Error> {
    set_paused(pool, bus, wake, true).await?;
    let google = google::revoke_stored(pool, google).await?;
    sqlx::query(&format!(
        "TRUNCATE {} RESTART IDENTITY",
        USER_TABLES.join(", ")
    ))
    .execute(pool)
    .await?;
    bus.publish(Event::DataDeleted);
    Ok(google)
}
