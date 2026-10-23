//! `tidy_application`, the sample workflow that exercises every guardrail without a model
//! call: `inspect` reads an application (`read`), `note` adds a note to its timeline
//! (`write`, undoable) unless the run's confidence sends it to the review queue, and
//! `draft` drafts a follow-up message (`external`), which waits for the user's approval.
//! The draft is only returned, never sent: agents draft, the user sends.
//!
//! `POST /applications/{id}/tidy` starts it.

use rocket::State;
use rocket::http::Status;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};

use crate::applications::timeline::{self, EventKind};
use crate::applications::{
    ApiError, FieldError, JsonContent, MAX_NOTES, REQUIRED, ValidationErrors, parsed, text,
};
use crate::evals::Suite;
use crate::events::EventBus;
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::Wake;
use crate::runs::{self, Run, Runtime, StepContext, StepError, Workflow};
use crate::tools::{Permission, Tool, ToolContext, ToolError, UndoError};

/// The workflow's name.
pub const WORKFLOW: &str = "tidy_application";
/// The review item kind `note` uses.
pub const NOTE_REVIEW_KIND: &str = "tidy.note";
const OUT_OF_RANGE: &str = "Must be between 0 and 1";

/// A tool failure for a database error, which goes to the log rather than to the caller
/// (its text can name tables).
fn db_failed(what: &'static str) -> impl FnOnce(sqlx::Error) -> ToolError {
    move |err| {
        rocket::warn!("tidy: cannot {what}: {err}");
        ToolError::Failed(format!("cannot {what}"))
    }
}

fn missing(id: i64) -> ToolError {
    ToolError::Failed(format!("application {id} doesn't exist"))
}

fn application_id_schema() -> Value {
    json!({"type": "integer", "description": "The application's id"})
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationRef {
    pub application_id: i64,
}

/// What `get_application` returns.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ApplicationSummary {
    pub id: i64,
    pub company: String,
    pub title: String,
    pub status: String,
}

/// `read`: an application's id, company, title and status.
pub struct GetApplication;

#[rocket::async_trait]
impl Tool for GetApplication {
    const NAME: &'static str = "get_application";
    const DESCRIPTION: &'static str = "Reads an application's company, title and status";
    const PERMISSION: Permission = Permission::Read;
    type Input = ApplicationRef;
    type Output = ApplicationSummary;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"application_id": application_id_schema()},
            "required": ["application_id"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "integer"},
                "company": {"type": "string"},
                "title": {"type": "string"},
                "status": {"type": "string"}
            },
            "required": ["id", "company", "title", "status"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        ctx: &mut ToolContext<'_>,
        input: ApplicationRef,
    ) -> Result<ApplicationSummary, ToolError> {
        sqlx::query_as("SELECT id, company, title, status FROM applications WHERE id = $1")
            .bind(input.application_id)
            .fetch_optional(ctx.pool())
            .await
            .map_err(db_failed("read the application"))?
            .ok_or_else(|| missing(input.application_id))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    pub application_id: i64,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct NoteAdded {
    pub event_id: i64,
}

/// What `add_note`'s undo needs.
#[derive(Debug, Serialize, Deserialize)]
struct NoteUndo {
    event_id: i64,
    application_id: i64,
}

/// `write`: adds a note to an application's timeline. Undo deletes that note.
pub struct AddNote;

#[rocket::async_trait]
impl Tool for AddNote {
    const NAME: &'static str = "add_note";
    const DESCRIPTION: &'static str = "Adds a note to an application's timeline";
    const PERMISSION: Permission = Permission::Write;
    type Input = NoteInput;
    type Output = NoteAdded;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "application_id": application_id_schema(),
                "note": {"type": "string", "description": "The note's text"}
            },
            "required": ["application_id", "note"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"event_id": {"type": "integer"}},
            "required": ["event_id"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        ctx: &mut ToolContext<'_>,
        input: NoteInput,
    ) -> Result<NoteAdded, ToolError> {
        let id = input.application_id;
        let mut errors = Vec::new();
        let note = text(&mut errors, "note", Some(&input.note), MAX_NOTES);
        let Some(note) = note.filter(|_| errors.is_empty()) else {
            return Err(ToolError::Failed(format!(
                "the note must be 1 to {MAX_NOTES} characters"
            )));
        };
        let mut tx = ctx.pool().begin().await.map_err(db_failed("add a note"))?;
        // Locks the application, as every timeline writer does.
        let found: Option<String> =
            sqlx::query_scalar("SELECT status FROM applications WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_failed("add a note"))?;
        if found.is_none() {
            return Err(missing(id));
        }
        sqlx::query("UPDATE applications SET updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_failed("add a note"))?;
        let event = timeline::record(&mut tx, id, EventKind::Note, None, None, None, Some(&note))
            .await
            .map_err(db_failed("add a note"))?;
        tx.commit().await.map_err(db_failed("add a note"))?;
        ctx.set_undo(json!(NoteUndo {
            event_id: event.id,
            application_id: id,
        }));
        Ok(NoteAdded { event_id: event.id })
    }

    async fn undo(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError> {
        let NoteUndo {
            event_id,
            application_id,
        } = serde_json::from_value(data)
            .map_err(|_| UndoError::Conflict("the undo data doesn't fit add_note".into()))?;
        let deleted = sqlx::query(
            "DELETE FROM application_events WHERE id = $1 AND application_id = $2 \
             AND kind = 'note'",
        )
        .bind(event_id)
        .bind(application_id)
        .execute(&mut *conn)
        .await?;
        if deleted.rows_affected() == 0 {
            return Err(UndoError::Conflict(format!(
                "note {event_id} of application {application_id} is gone"
            )));
        }
        sqlx::query("UPDATE applications SET updated_at = now() WHERE id = $1")
            .bind(application_id)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftInput {
    pub application_id: i64,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct Draft {
    pub draft: String,
}

/// `external`: drafts a follow-up message to an application's company. It only returns
/// the draft and sends or writes nothing; it is `external` because sending is what a real
/// follow-up does, so it shows the approval gate.
pub struct DraftFollowUp;

#[rocket::async_trait]
impl Tool for DraftFollowUp {
    const NAME: &'static str = "draft_follow_up";
    const DESCRIPTION: &'static str =
        "Drafts a follow-up message to an application's company; needs the user's approval";
    const PERMISSION: Permission = Permission::External;
    type Input = DraftInput;
    type Output = Draft;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "application_id": application_id_schema(),
                "message": {"type": "string", "description": "The message to draft"}
            },
            "required": ["application_id", "message"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"draft": {"type": "string"}},
            "required": ["draft"],
            "additionalProperties": false
        })
    }

    async fn call(&self, ctx: &mut ToolContext<'_>, input: DraftInput) -> Result<Draft, ToolError> {
        let company: String = sqlx::query_scalar("SELECT company FROM applications WHERE id = $1")
            .bind(input.application_id)
            .fetch_optional(ctx.pool())
            .await
            .map_err(db_failed("read the application"))?
            .ok_or_else(|| missing(input.application_id))?;
        Ok(Draft {
            draft: format!("{} (to {company})", input.message),
        })
    }
}

/// The workflow. Its run input is `{"application_id", "note", "message", "confidence"}`;
/// each step passes its input through and adds to it, and `draft`'s output is the run's.
pub struct TidyApplication;

fn field<'a>(input: &'a Value, name: &str) -> Result<&'a Value, StepError> {
    input
        .get(name)
        .ok_or_else(|| StepError::Fail(format!("the run input has no {name:?}")))
}

fn string_field<'a>(input: &'a Value, name: &str) -> Result<&'a str, StepError> {
    field(input, name)?
        .as_str()
        .ok_or_else(|| StepError::Fail(format!("the run input's {name:?} must be a string")))
}

#[rocket::async_trait]
impl Workflow for TidyApplication {
    fn name(&self) -> &'static str {
        WORKFLOW
    }

    fn steps(&self) -> &'static [&'static str] {
        &["inspect", "note", "draft"]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        let application_id = field(&input, "application_id")?.as_i64().ok_or_else(|| {
            StepError::Fail("the run input's application_id must be an integer".into())
        })?;
        let mut output = input.clone();
        let Value::Object(added) = &mut output else {
            return Err(StepError::Fail("the run input must be an object".into()));
        };
        match step {
            "inspect" => {
                let application = ctx
                    .tool(
                        GetApplication::NAME,
                        json!({"application_id": application_id}),
                    )
                    .await?;
                added.insert("application".into(), application);
            }
            "note" => {
                let note = string_field(&input, "note")?;
                let confidence = field(&input, "confidence")?.as_f64().ok_or_else(|| {
                    StepError::Fail("the run input's confidence must be a number".into())
                })?;
                let payload = json!({"application_id": application_id, "note": note});
                let review = ctx
                    .review_if_unsure(NOTE_REVIEW_KIND, confidence, payload)
                    .await
                    .map_err(|err| {
                        StepError::Retry(format!("cannot store a review item: {err}"))
                    })?;
                match review {
                    Some(id) => {
                        added.insert("review_item_id".into(), json!(id));
                    }
                    None => {
                        let added_note = ctx
                            .tool(
                                AddNote::NAME,
                                json!({"application_id": application_id, "note": note}),
                            )
                            .await?;
                        added.insert("note_event_id".into(), added_note["event_id"].clone());
                    }
                }
            }
            "draft" => {
                let message = string_field(&input, "message")?;
                return Ok(ctx
                    .tool(
                        DraftFollowUp::NAME,
                        json!({"application_id": application_id, "message": message}),
                    )
                    .await?);
            }
            other => return Err(StepError::Fail(format!("no step is named {other:?}"))),
        }
        Ok(output)
    }
}

/// A `tidy_application` eval case's `setup`: `{"application": {...}}` inserts one, and
/// `{}` or `null` inserts nothing.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalSetup {
    #[serde(default)]
    application: Option<EvalApplication>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalApplication {
    company: String,
    title: String,
    /// The column default, `sourced`, when absent.
    #[serde(default)]
    status: Option<String>,
}

/// The eval suite for `tidy_application` (dataset `api/evals/tidy_application.json`).
/// Setup inserts the case's application and returns `{"application_id": <id>}`; without
/// one it returns no variables.
pub struct TidyEval;

#[rocket::async_trait]
impl Suite for TidyEval {
    fn workflow(&self) -> &'static str {
        WORKFLOW
    }

    async fn setup(&self, pool: &PgPool, setup: &Value) -> Result<Map<String, Value>, String> {
        let setup = if setup.is_null() {
            EvalSetup::default()
        } else {
            EvalSetup::deserialize(setup).map_err(|err| format!("invalid setup: {err}"))?
        };
        let mut vars = Map::new();
        if let Some(application) = setup.application {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO applications (company, title, status) \
                 VALUES ($1, $2, COALESCE($3, 'sourced')) RETURNING id",
            )
            .bind(&application.company)
            .bind(&application.title)
            .bind(&application.status)
            .fetch_one(pool)
            .await
            .map_err(|err| format!("cannot insert the application: {err}"))?;
            vars.insert("application_id".into(), json!(id));
        }
        Ok(vars)
    }
}

/// The `POST /applications/{id}/tidy` body.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TidyRequest {
    /// The note to add to the timeline. Required; trimmed, at most 20,000 characters.
    #[serde(default)]
    #[schema(required = true)]
    pub note: String,
    /// The follow-up message to draft. Required; trimmed, at most 20,000 characters.
    #[serde(default)]
    #[schema(required = true)]
    pub message: String,
    /// How sure the caller is of the note, 0 to 1 (default 1). Below 0.8 the note goes to
    /// the review queue instead of the timeline.
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// The run input for a valid request, or every failing field.
fn validate(application_id: i64, body: &TidyRequest) -> Result<Value, ValidationErrors> {
    let mut errors = Vec::new();
    let mut required = |field: &str, value: &str| {
        let value = text(&mut errors, field, Some(value), MAX_NOTES);
        if value.is_none() {
            errors.push(FieldError {
                field: field.into(),
                message: REQUIRED.into(),
            });
        }
        value.unwrap_or_default()
    };
    let note = required("note", &body.note);
    let message = required("message", &body.message);
    let confidence = body.confidence.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&confidence) {
        errors.push(FieldError {
            field: "confidence".into(),
            message: OUT_OF_RANGE.into(),
        });
    }
    if errors.is_empty() {
        Ok(json!({
            "application_id": application_id,
            "note": note,
            "message": message,
            "confidence": confidence,
        }))
    } else {
        Err(ValidationErrors { errors })
    }
}

/// Starts the `tidy_application` sample workflow for an application.
#[utoipa::path(
    post,
    path = "/applications/{id}/tidy",
    tag = "runs",
    operation_id = "start_tidy",
    params(("id" = i64, Path, description = "The application's id")),
    request_body = TidyRequest,
    responses(
        (status = 201, description = "The queued `tidy_application` run", body = Run),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`note` or `message` is blank or too long, or `confidence` is outside 0..=1", body = ValidationErrors),
        (status = 500, description = "The run could not be started"),
    )
)]
#[rocket::post("/applications/<id>/tidy", data = "<body>")]
#[allow(clippy::too_many_arguments)]
pub async fn start_tidy(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    runtime: &State<Runtime>,
    body: Result<Json<TidyRequest>, json::Error<'_>>,
) -> Result<(Status, Json<Run>), ApiError> {
    let input = validate(id, &parsed(body)?).map_err(ApiError::Invalid)?;
    timeline::ensure_exists(pool, id).await?;
    let run = runs::start(pool, bus, wake, runtime, WORKFLOW, input)
        .await
        .map_err(|err| {
            rocket::warn!("tidy: cannot start a run: {err}");
            ApiError::Internal
        })?;
    Ok((Status::Created, Json(run)))
}
