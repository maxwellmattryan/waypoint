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
    /// An interview the user approved was added to Google Calendar; `body` names its
    /// summary and time, and `source_email` the email that proposed it.
    InterviewScheduled,
}

impl EventKind {
    /// Every kind.
    pub const ALL: [Self; 9] = [
        Self::Created,
        Self::StatusChanged,
        Self::StageAdded,
        Self::StageCompleted,
        Self::StageReopened,
        Self::StageRemoved,
        Self::Note,
        Self::RejectionFeedback,
        Self::InterviewScheduled,
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
            Self::InterviewScheduled => "interview_scheduled",
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

/// The email that caused an event, as the timeline shows it.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct SourceEmail {
    pub id: i64,
    pub subject: String,
    #[schema(required = true)]
    pub from_name: Option<String>,
    #[schema(required = true)]
    pub from_address: Option<String>,
    #[schema(required = true)]
    pub sent_at: Option<DateTime<Utc>>,
}

/// One `application_events` row. Which of the optional fields are set depends on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ApplicationEvent {
    pub id: i64,
    pub application_id: i64,
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
    /// The email that caused the event (an email-driven status or stage change); `null`
    /// for every other event and once the email is deleted.
    #[schema(required = true)]
    pub source_email: Option<SourceEmail>,
    pub created_at: DateTime<Utc>,
}

/// An event joined with its source email, as one flat row.
#[derive(sqlx::FromRow)]
struct EventRow {
    id: i64,
    application_id: i64,
    #[sqlx(try_from = "String")]
    kind: EventKind,
    from_status: Option<ApplicationStatus>,
    to_status: Option<ApplicationStatus>,
    stage_name: Option<String>,
    body: Option<String>,
    created_at: DateTime<Utc>,
    source_email_id: Option<i64>,
    source_subject: Option<String>,
    source_from_name: Option<String>,
    source_from_address: Option<String>,
    source_sent_at: Option<DateTime<Utc>>,
}

impl From<EventRow> for ApplicationEvent {
    fn from(row: EventRow) -> Self {
        let source_email = match (row.source_email_id, row.source_subject) {
            (Some(id), Some(subject)) => Some(SourceEmail {
                id,
                subject,
                from_name: row.source_from_name,
                from_address: row.source_from_address,
                sent_at: row.source_sent_at,
            }),
            _ => None,
        };
        Self {
            id: row.id,
            application_id: row.application_id,
            kind: row.kind,
            from_status: row.from_status,
            to_status: row.to_status,
            stage_name: row.stage_name,
            body: row.body,
            source_email,
            created_at: row.created_at,
        }
    }
}

/// The select list of an [`EventRow`] from `ev` (events) left-joined to `m` (emails).
const SELECT: &str = "SELECT ev.id, ev.application_id, ev.kind, ev.from_status, ev.to_status, \
                      ev.stage_name, ev.body, ev.created_at, m.id AS source_email_id, \
                      m.subject AS source_subject, m.from_name AS source_from_name, \
                      m.from_address AS source_from_address, m.sent_at AS source_sent_at";

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
    insert(conn, application_id, kind, from, to, stage_name, body, None).await
}

/// [`record`] for an event an email caused: the event names `source_email_id`, or no
/// email when that one no longer exists.
pub(crate) async fn record_sourced(
    conn: &mut PgConnection,
    application_id: i64,
    kind: EventKind,
    from: Option<ApplicationStatus>,
    to: Option<ApplicationStatus>,
    stage_name: Option<&str>,
    source_email_id: Option<i64>,
) -> Result<ApplicationEvent, sqlx::Error> {
    insert(
        conn,
        application_id,
        kind,
        from,
        to,
        stage_name,
        None,
        source_email_id,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert(
    conn: &mut PgConnection,
    application_id: i64,
    kind: EventKind,
    from: Option<ApplicationStatus>,
    to: Option<ApplicationStatus>,
    stage_name: Option<&str>,
    body: Option<&str>,
    source_email_id: Option<i64>,
) -> Result<ApplicationEvent, sqlx::Error> {
    sqlx::query_as::<_, EventRow>(&format!(
        "WITH ev AS (INSERT INTO application_events \
           (application_id, kind, from_status, to_status, stage_name, body, source_email_id) \
           VALUES ($1, $2, $3, $4, $5, $6, (SELECT id FROM emails WHERE id = $7)) \
           RETURNING *) \
         {SELECT} FROM ev LEFT JOIN emails m ON m.id = ev.source_email_id"
    ))
    .bind(application_id)
    .bind(kind.as_str())
    .bind(from.map(ApplicationStatus::as_str))
    .bind(to.map(ApplicationStatus::as_str))
    .bind(stage_name)
    .bind(body)
    .bind(source_email_id)
    .fetch_one(conn)
    .await
    .map(ApplicationEvent::from)
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
    sqlx::query_as::<_, EventRow>(&format!(
        "{SELECT} FROM application_events ev LEFT JOIN emails m ON m.id = ev.source_email_id \
         WHERE ev.application_id = $1 ORDER BY ev.created_at DESC, ev.id DESC"
    ))
    .bind(id)
    .fetch_all(pool.inner())
    .await
    .map(|rows| Json(rows.into_iter().map(ApplicationEvent::from).collect()))
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
