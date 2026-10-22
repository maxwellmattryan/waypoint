//! An application's timeline: the append-only `application_events` log. Every write that
//! changes an application's status or stages records an event here through [`record`],
//! in the same transaction, and the user adds notes and rejection feedback directly.
//!
//! Every writer starts its transaction with [`lock_application`], so concurrent writes to
//! one application run one after another, and sets `updated_at` with [`touch`] (or its own
//! UPDATE), so activity moves the application up the list.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};

use super::{
    ApiError, ApplicationStatus, FieldError, JsonContent, MAX_NOTES, REQUIRED, ValidationErrors,
    internal, parsed, text,
};
use crate::host::{LocalHost, NotCrossSite};

const FEEDBACK_NOT_REJECTED: &str = "Only for a rejected application";

/// What an event records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The application was added; `to_status` is its starting status (null for
    /// applications that existed before the timeline).
    Created,
    /// `from_status` → `to_status`.
    StatusChanged,
    /// `stage_name` was added.
    StageAdded,
    StageCompleted,
    StageReopened,
    StageRemoved,
    /// A note; the text is `body`.
    Note,
    /// What the company said when it rejected the application; the text is `body`.
    RejectionFeedback,
}

impl EventKind {
    /// Every kind.
    pub const ALL: [Self; 8] = [
        Self::Created,
        Self::StatusChanged,
        Self::StageAdded,
        Self::StageCompleted,
        Self::StageReopened,
        Self::StageRemoved,
        Self::Note,
        Self::RejectionFeedback,
    ];

    /// The wire and database name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::StatusChanged => "status_changed",
            Self::StageAdded => "stage_added",
            Self::StageCompleted => "stage_completed",
            Self::StageReopened => "stage_reopened",
            Self::StageRemoved => "stage_removed",
            Self::Note => "note",
            Self::RejectionFeedback => "rejection_feedback",
        }
    }
}

impl TryFrom<String> for EventKind {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| format!("unknown event kind {value:?}"))
    }
}

/// One `application_events` row. Which of the optional fields are set depends on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ApplicationEvent {
    pub id: i64,
    pub application_id: i64,
    #[sqlx(try_from = "String")]
    pub kind: EventKind,
    // The `Option` fields are always serialized (as `null` when empty), so they are
    // required in the schema.
    /// Set for `status_changed`.
    #[schema(required = true)]
    pub from_status: Option<ApplicationStatus>,
    /// Set for `status_changed` and (except for backfilled rows) `created`.
    #[schema(required = true)]
    pub to_status: Option<ApplicationStatus>,
    /// The stage's name when the event was written, for the `stage_*` kinds.
    #[schema(required = true)]
    pub stage_name: Option<String>,
    /// The text of a `note` or `rejection_feedback`.
    #[schema(required = true)]
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, application_id, kind, from_status, to_status, stage_name, body, \
                       created_at";

/// The kinds of entry the user writes directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Note,
    /// Only for an application whose status is `rejected`.
    RejectionFeedback,
}

impl From<EntryKind> for EventKind {
    fn from(kind: EntryKind) -> Self {
        match kind {
            EntryKind::Note => Self::Note,
            EntryKind::RejectionFeedback => Self::RejectionFeedback,
        }
    }
}

/// The `POST /applications/{id}/events` body.
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct NewEntry {
    pub kind: EntryKind,
    /// Required; trimmed, at most 20,000 characters.
    #[serde(default)]
    #[schema(required = true)]
    pub body: String,
}

/// Appends one event to an application's timeline.
pub(crate) async fn record(
    conn: &mut PgConnection,
    application_id: i64,
    kind: EventKind,
    from: Option<ApplicationStatus>,
    to: Option<ApplicationStatus>,
    stage_name: Option<&str>,
    body: Option<&str>,
) -> Result<ApplicationEvent, sqlx::Error> {
    sqlx::query_as(&format!(
        "INSERT INTO application_events \
         (application_id, kind, from_status, to_status, stage_name, body) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING {COLUMNS}"
    ))
    .bind(application_id)
    .bind(kind.as_str())
    .bind(from.map(ApplicationStatus::as_str))
    .bind(to.map(ApplicationStatus::as_str))
    .bind(stage_name)
    .bind(body)
    .fetch_one(conn)
    .await
}

/// Locks an application's row for the rest of the transaction and returns its status, or
/// 404 when there is no such application.
pub(crate) async fn lock_application(
    conn: &mut PgConnection,
    id: i64,
) -> Result<ApplicationStatus, ApiError> {
    sqlx::query_scalar("SELECT status FROM applications WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(conn)
        .await
        .map_err(internal("lock an application"))?
        .ok_or(ApiError::NotFound)
}

/// Sets an application's `updated_at` to now.
pub(crate) async fn touch(conn: &mut PgConnection, id: i64) -> Result<(), ApiError> {
    sqlx::query("UPDATE applications SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(conn)
        .await
        .map_err(internal("update an application"))?;
    Ok(())
}

/// 404 unless an application with this id exists. For the read routes, which lock nothing.
pub(crate) async fn ensure_exists(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM applications WHERE id = $1)")
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(internal("read an application"))?;
    if exists {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

/// An application's timeline, newest first.
#[utoipa::path(
    get,
    path = "/applications/{id}/events",
    tag = "applications",
    operation_id = "list_application_events",
    params(("id" = i64, Path, description = "The application's id")),
    responses(
        (status = 200, description = "Every event of the application, newest `created_at` first, ties by `id` descending", body = [ApplicationEvent]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No application has this id"),
        (status = 500, description = "The timeline could not be read"),
    )
)]
#[rocket::get("/applications/<id>/events")]
pub async fn list_events(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<Vec<ApplicationEvent>>, ApiError> {
    ensure_exists(pool, id).await?;
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM application_events WHERE application_id = $1 \
         ORDER BY created_at DESC, id DESC"
    ))
    .bind(id)
    .fetch_all(pool.inner())
    .await
    .map(Json)
    .map_err(internal("read a timeline"))
}

/// Adds a note, or rejection feedback for a rejected application, to the timeline.
#[utoipa::path(
    post,
    path = "/applications/{id}/events",
    tag = "applications",
    operation_id = "add_application_entry",
    params(("id" = i64, Path, description = "The application's id")),
    request_body = NewEntry,
    responses(
        (status = 201, description = "The stored event", body = ApplicationEvent),
        (status = 400, description = "The body is not valid JSON for this route, or `kind` is unknown", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`body` is blank or too long, or `kind` is `rejection_feedback` and the application isn't rejected", body = ValidationErrors),
        (status = 500, description = "The entry could not be stored"),
    )
)]
#[rocket::post("/applications/<id>/events", data = "<body>")]
pub async fn add_entry(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    body: Result<Json<NewEntry>, json::Error<'_>>,
) -> Result<(Status, Json<ApplicationEvent>), ApiError> {
    let entry = parsed(body)?;
    let mut errors = Vec::new();
    let text = text(&mut errors, "body", Some(&entry.body), MAX_NOTES);
    if text.is_none() {
        errors.push(FieldError {
            field: "body".into(),
            message: REQUIRED.into(),
        });
    }
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a transaction"))?;
    let status = lock_application(&mut tx, id).await?;
    if entry.kind == EntryKind::RejectionFeedback && status != ApplicationStatus::Rejected {
        return Err(ApiError::Invalid(ValidationErrors {
            errors: vec![FieldError {
                field: "kind".into(),
                message: FEEDBACK_NOT_REJECTED.into(),
            }],
        }));
    }
    touch(&mut tx, id).await?;
    let event = record(
        &mut tx,
        id,
        entry.kind.into(),
        None,
        None,
        None,
        text.as_deref(),
    )
    .await
    .map_err(internal("store a timeline entry"))?;
    tx.commit()
        .await
        .map_err(internal("store a timeline entry"))?;
    Ok((Status::Created, Json(event)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips_through_text() {
        for kind in EventKind::ALL {
            assert_eq!(EventKind::try_from(kind.as_str().to_string()), Ok(kind));
        }
        assert!(EventKind::try_from("edited".to_string()).is_err());
    }
}
