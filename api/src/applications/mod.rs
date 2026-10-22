//! Job applications: one `applications` row per role the user is tracking, with CRUD
//! routes and a status route. Creating an application and changing its status also
//! write to its timeline ([`timeline`]); its interview stages live in [`stages`].
//!
//! Input is normalised here and nowhere else: text is trimmed, a blank optional field
//! becomes `null`, lengths are counted in characters and `url` must be an absolute http or
//! https URL. A body that doesn't parse is a 400; a body that parses but breaks a rule is
//! a 422 naming every failing field.

pub mod stages;
pub mod timeline;

use std::fmt;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::{self, Responder};
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgTypeInfo, PgValueRef};
use sqlx::{Decode, PgPool, Postgres, Type};

use crate::host::{LocalHost, NotCrossSite};
use timeline::EventKind;

/// Most characters in `company`, `title`, `location`, `comp` and `source`.
pub const MAX_SHORT: usize = 200;
/// Most characters in `url`.
pub const MAX_URL: usize = 2048;
/// Most characters in `notes`.
pub const MAX_NOTES: usize = 20_000;

pub(crate) const REQUIRED: &str = "Required";
const FEEDBACK_NOT_REJECTED: &str = "Only when the status is rejected";
const BAD_URL: &str = "Must be an http or https URL";
const BAD_BODY: &str = "The request body is not valid JSON for this route";

/// Where an application is in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationStatus {
    /// Found, not applied to yet.
    Sourced,
    Applied,
    Interviewing,
    Offer,
    Rejected,
}

impl ApplicationStatus {
    /// Every status, in board order.
    pub const ALL: [Self; 5] = [
        Self::Sourced,
        Self::Applied,
        Self::Interviewing,
        Self::Offer,
        Self::Rejected,
    ];

    /// The wire and database name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sourced => "sourced",
            Self::Applied => "applied",
            Self::Interviewing => "interviewing",
            Self::Offer => "offer",
            Self::Rejected => "rejected",
        }
    }
}

impl TryFrom<String> for ApplicationStatus {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
            .ok_or_else(|| format!("unknown application status {value:?}"))
    }
}

// Decoding straight from a TEXT column lets nullable status columns read as
// `Option<ApplicationStatus>`.
impl Type<Postgres> for ApplicationStatus {
    fn type_info() -> PgTypeInfo {
        <String as Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <String as Type<Postgres>>::compatible(ty)
    }
}

impl<'r> Decode<'r, Postgres> for ApplicationStatus {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let text = <String as Decode<Postgres>>::decode(value)?;
        Ok(Self::try_from(text)?)
    }
}

/// One `applications` row: the body of every route that returns an application.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Application {
    pub id: i64,
    pub company: String,
    pub title: String,
    // The `Option` fields are always serialized (as `null` when empty), so they are
    // required in the schema.
    /// The job posting, an absolute http or https URL.
    #[schema(required = true)]
    pub url: Option<String>,
    #[schema(required = true)]
    pub location: Option<String>,
    /// Compensation as free text, e.g. "$180k–$210k + equity".
    #[schema(required = true)]
    pub comp: Option<String>,
    /// Where the role came from as free text, e.g. "Referral".
    #[schema(required = true)]
    pub source: Option<String>,
    #[schema(required = true)]
    pub notes: Option<String>,
    #[sqlx(try_from = "String")]
    pub status: ApplicationStatus,
    /// When `status` last changed (the creation time until then).
    pub status_changed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, company, title, url, location, comp, source, notes, status, \
                       status_changed_at, created_at, updated_at";

/// The `PUT /applications/{id}` body: every text field, replacing the stored ones. An
/// omitted optional field is cleared. Unknown keys, `status` included, are ignored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct ApplicationInput {
    /// Required; a missing or blank value is a 422.
    #[serde(default)]
    #[schema(required = true)]
    pub company: String,
    /// Required; a missing or blank value is a 422.
    #[serde(default)]
    #[schema(required = true)]
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub comp: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// The `POST /applications` body: the [`ApplicationInput`] fields plus an optional
/// starting status (`sourced` when omitted).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct NewApplication {
    /// Required; a missing or blank value is a 422.
    #[serde(default)]
    #[schema(required = true)]
    pub company: String,
    /// Required; a missing or blank value is a 422.
    #[serde(default)]
    #[schema(required = true)]
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub comp: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub status: Option<ApplicationStatus>,
}

/// The `PUT /applications/{id}/status` body.
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct StatusChange {
    pub status: ApplicationStatus,
    /// Rejection feedback, recorded as a `rejection_feedback` event. Allowed only with
    /// `rejected`; blank means none. At most 20,000 characters.
    #[serde(default)]
    pub feedback: Option<String>,
}

/// One rule a field broke.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct FieldError {
    /// The JSON field name, or `body` when the body itself didn't parse.
    pub field: String,
    pub message: String,
}

/// The 400 and 422 body: every failing field at once.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ValidationErrors {
    pub errors: Vec<FieldError>,
}

impl ValidationErrors {
    fn body() -> Self {
        Self {
            errors: vec![FieldError {
                field: "body".into(),
                message: BAD_BODY.into(),
            }],
        }
    }
}

/// Why a route failed. 404 and 500 go to Rocket's catchers, like `jobs::show`.
#[derive(Debug)]
pub enum ApiError {
    /// 422 with the failing fields.
    Invalid(ValidationErrors),
    /// 400: the body didn't parse.
    BadRequest,
    NotFound,
    Internal,
}

impl<'r> Responder<'r, 'static> for ApiError {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        match self {
            Self::Invalid(errors) => (Status::UnprocessableEntity, Json(errors)).respond_to(req),
            Self::BadRequest => {
                (Status::BadRequest, Json(ValidationErrors::body())).respond_to(req)
            }
            Self::NotFound => Err(Status::NotFound),
            Self::Internal => Err(Status::InternalServerError),
        }
    }
}

/// Logs a database error for `what` and turns it into a 500.
pub(crate) fn internal(what: impl fmt::Display) -> impl FnOnce(sqlx::Error) -> ApiError {
    move |err| {
        rocket::warn!("applications: cannot {what}: {err}");
        ApiError::Internal
    }
}

/// Unwraps a JSON body, or answers 400 without echoing any of it.
pub(crate) fn parsed<T>(body: Result<Json<T>, json::Error<'_>>) -> Result<T, ApiError> {
    match body {
        Ok(Json(value)) => Ok(value),
        Err(json::Error::Parse(_, err)) => {
            // serde's message can quote the input (an unknown variant), so log only where.
            rocket::warn!(
                "applications: body did not parse ({:?} at line {} column {})",
                err.classify(),
                err.line(),
                err.column()
            );
            Err(ApiError::BadRequest)
        }
        Err(json::Error::Io(err)) => {
            rocket::warn!("applications: body could not be read: {}", err.kind());
            Err(ApiError::BadRequest)
        }
    }
}

/// Admits only a `Content-Type: application/json` body. Taken after [`NotCrossSite`], so
/// a form body is a 403 and any other type (or none) is a 415; neither writes anything.
pub struct JsonContent;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for JsonContent {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        if req.content_type().is_some_and(|ct| ct.is_json()) {
            Outcome::Success(Self)
        } else {
            Outcome::Error((Status::UnsupportedMediaType, ()))
        }
    }
}

/// Validated input, ready to store.
#[derive(Debug, Clone, PartialEq)]
struct Fields {
    company: String,
    title: String,
    url: Option<String>,
    location: Option<String>,
    comp: Option<String>,
    source: Option<String>,
    notes: Option<String>,
}

/// Trims a value and turns a blank one into `None`, recording a length error.
pub(crate) fn text(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: Option<&str>,
    max: usize,
) -> Option<String> {
    let value = value.map(str::trim).filter(|v| !v.is_empty())?;
    if value.chars().count() > max {
        errors.push(FieldError {
            field: field.into(),
            message: format!("At most {max} characters"),
        });
    }
    Some(value.to_string())
}

/// Whether `value` is an absolute http or https URL with a host.
fn is_web_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty())
    })
}

fn validate(input: &ApplicationInput) -> Result<Fields, ValidationErrors> {
    let mut errors = Vec::new();
    let required = |errors: &mut Vec<FieldError>, field: &str, value: &str| {
        let value = text(errors, field, Some(value), MAX_SHORT);
        if value.is_none() {
            errors.push(FieldError {
                field: field.into(),
                message: REQUIRED.into(),
            });
        }
        value.unwrap_or_default()
    };
    let company = required(&mut errors, "company", &input.company);
    let title = required(&mut errors, "title", &input.title);

    let before = errors.len();
    let url = text(&mut errors, "url", input.url.as_deref(), MAX_URL);
    if errors.len() == before && url.as_deref().is_some_and(|url| !is_web_url(url)) {
        errors.push(FieldError {
            field: "url".into(),
            message: BAD_URL.into(),
        });
    }
    let location = text(
        &mut errors,
        "location",
        input.location.as_deref(),
        MAX_SHORT,
    );
    let comp = text(&mut errors, "comp", input.comp.as_deref(), MAX_SHORT);
    let source = text(&mut errors, "source", input.source.as_deref(), MAX_SHORT);
    let notes = text(&mut errors, "notes", input.notes.as_deref(), MAX_NOTES);

    if errors.is_empty() {
        Ok(Fields {
            company,
            title,
            url,
            location,
            comp,
            source,
            notes,
        })
    } else {
        Err(ValidationErrors { errors })
    }
}

/// Every application, the most recently updated first.
#[utoipa::path(
    get,
    path = "/applications",
    tag = "applications",
    operation_id = "list_applications",
    responses(
        (status = 200, description = "Every application, newest `updated_at` first, ties by `id` descending", body = [Application]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The applications could not be read"),
    )
)]
#[rocket::get("/applications")]
pub async fn list(
    _host: LocalHost,
    pool: &State<PgPool>,
) -> Result<Json<Vec<Application>>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM applications ORDER BY updated_at DESC, id DESC"
    ))
    .fetch_all(pool.inner())
    .await
    .map(Json)
    .map_err(internal("list applications"))
}

/// Adds an application.
#[utoipa::path(
    post,
    path = "/applications",
    tag = "applications",
    operation_id = "create_application",
    request_body = NewApplication,
    responses(
        (status = 201, description = "The stored application", body = Application),
        (status = 400, description = "The body is not valid JSON for this route, or `status` is unknown", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "One or more fields are invalid", body = ValidationErrors),
        (status = 500, description = "The application could not be stored"),
    )
)]
#[rocket::post("/applications", data = "<body>")]
pub async fn create(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    pool: &State<PgPool>,
    body: Result<Json<NewApplication>, json::Error<'_>>,
) -> Result<(Status, Json<Application>), ApiError> {
    let body = parsed(body)?;
    let status = body.status.unwrap_or(ApplicationStatus::Sourced);
    let input = ApplicationInput {
        company: body.company,
        title: body.title,
        url: body.url,
        location: body.location,
        comp: body.comp,
        source: body.source,
        notes: body.notes,
    };
    let fields = validate(&input).map_err(ApiError::Invalid)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a transaction"))?;
    let application: Application = sqlx::query_as(&format!(
        "INSERT INTO applications (company, title, url, location, comp, source, notes, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {COLUMNS}"
    ))
    .bind(fields.company)
    .bind(fields.title)
    .bind(fields.url)
    .bind(fields.location)
    .bind(fields.comp)
    .bind(fields.source)
    .bind(fields.notes)
    .bind(status.as_str())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal("store an application"))?;
    timeline::record(
        &mut tx,
        application.id,
        EventKind::Created,
        None,
        Some(status),
        None,
        None,
    )
    .await
    .map_err(internal("record an application's creation"))?;
    tx.commit()
        .await
        .map_err(internal("store an application"))?;
    Ok((Status::Created, Json(application)))
}

/// One application.
#[utoipa::path(
    get,
    path = "/applications/{id}",
    tag = "applications",
    operation_id = "get_application",
    params(("id" = i64, Path, description = "The application's id")),
    responses(
        (status = 200, description = "The application", body = Application),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No application has this id"),
        (status = 500, description = "The application could not be read"),
    )
)]
#[rocket::get("/applications/<id>")]
pub async fn show(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<Application>, ApiError> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM applications WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(internal("read an application"))?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Replaces an application's text fields. `status` and `status_changed_at` are left
/// alone, even when the body carries a `status`.
#[utoipa::path(
    put,
    path = "/applications/{id}",
    tag = "applications",
    operation_id = "update_application",
    params(("id" = i64, Path, description = "The application's id")),
    request_body = ApplicationInput,
    responses(
        (status = 200, description = "The updated application", body = Application),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "One or more fields are invalid", body = ValidationErrors),
        (status = 500, description = "The application could not be stored"),
    )
)]
#[rocket::put("/applications/<id>", data = "<body>")]
pub async fn update(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    body: Result<Json<ApplicationInput>, json::Error<'_>>,
) -> Result<Json<Application>, ApiError> {
    let fields = validate(&parsed(body)?).map_err(ApiError::Invalid)?;
    sqlx::query_as(&format!(
        "UPDATE applications SET company = $2, title = $3, url = $4, location = $5, \
         comp = $6, source = $7, notes = $8, updated_at = now() \
         WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(fields.company)
    .bind(fields.title)
    .bind(fields.url)
    .bind(fields.location)
    .bind(fields.comp)
    .bind(fields.source)
    .bind(fields.notes)
    .fetch_optional(pool.inner())
    .await
    .map_err(internal("update an application"))?
    .map(Json)
    .ok_or(ApiError::NotFound)
}

/// Sets an application's status. The single place status changes: when the status
/// differs, `status_changed_at` and `updated_at` move and a `status_changed` event is
/// written. Non-blank `feedback` (only with `rejected`) also writes a `rejection_feedback`
/// event and moves `updated_at`, even if the application was already rejected. The same
/// status without feedback is a no-op that returns the row unchanged.
#[utoipa::path(
    put,
    path = "/applications/{id}/status",
    tag = "applications",
    operation_id = "set_application_status",
    params(("id" = i64, Path, description = "The application's id")),
    request_body = StatusChange,
    responses(
        (status = 200, description = "The application with its new status", body = Application),
        (status = 400, description = "The body is not valid JSON for this route, or `status` is unknown", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`feedback` is too long, or given with a status other than `rejected`", body = ValidationErrors),
        (status = 500, description = "The status could not be stored"),
    )
)]
#[rocket::put("/applications/<id>/status", data = "<body>")]
pub async fn set_status(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    body: Result<Json<StatusChange>, json::Error<'_>>,
) -> Result<Json<Application>, ApiError> {
    let StatusChange { status, feedback } = parsed(body)?;
    let mut errors = Vec::new();
    let feedback = text(&mut errors, "feedback", feedback.as_deref(), MAX_NOTES);
    if errors.is_empty() && feedback.is_some() && status != ApplicationStatus::Rejected {
        errors.push(FieldError {
            field: "feedback".into(),
            message: FEEDBACK_NOT_REJECTED.into(),
        });
    }
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(internal("start a transaction"))?;
    let current = timeline::lock_application(&mut tx, id).await?;
    if current != status {
        sqlx::query(
            "UPDATE applications SET status = $2, status_changed_at = now(), \
             updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(status.as_str())
        .execute(&mut *tx)
        .await
        .map_err(internal("set an application's status"))?;
        timeline::record(
            &mut tx,
            id,
            EventKind::StatusChanged,
            Some(current),
            Some(status),
            None,
            None,
        )
        .await
        .map_err(internal("record a status change"))?;
    }
    if let Some(feedback) = feedback {
        if current == status {
            timeline::touch(&mut tx, id).await?;
        }
        timeline::record(
            &mut tx,
            id,
            EventKind::RejectionFeedback,
            None,
            None,
            None,
            Some(&feedback),
        )
        .await
        .map_err(internal("record rejection feedback"))?;
    }
    let application = sqlx::query_as(&format!("SELECT {COLUMNS} FROM applications WHERE id = $1"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal("read an application"))?;
    tx.commit()
        .await
        .map_err(internal("set an application's status"))?;
    Ok(Json(application))
}

/// Deletes an application permanently.
#[utoipa::path(
    delete,
    path = "/applications/{id}",
    tag = "applications",
    operation_id = "delete_application",
    params(("id" = i64, Path, description = "The application's id")),
    responses(
        (status = 204, description = "The application is deleted"),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No application has this id"),
        (status = 500, description = "The application could not be deleted"),
    )
)]
#[rocket::delete("/applications/<id>")]
pub async fn delete(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Status, ApiError> {
    let done = sqlx::query("DELETE FROM applications WHERE id = $1")
        .bind(id)
        .execute(pool.inner())
        .await
        .map_err(internal("delete an application"))?;
    if done.rows_affected() == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(Status::NoContent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_trims_and_nulls_blanks() {
        let input = ApplicationInput {
            company: "  Acme ".into(),
            title: "Engineer".into(),
            url: Some(" https://acme.example/jobs/1 ".into()),
            location: Some("   ".into()),
            ..Default::default()
        };
        let fields = validate(&input).unwrap();
        assert_eq!(fields.company, "Acme");
        assert_eq!(fields.url.as_deref(), Some("https://acme.example/jobs/1"));
        assert_eq!(fields.location, None);
    }

    #[test]
    fn url_must_be_absolute_http() {
        for (url, ok) in [
            ("https://a.example", true),
            ("http://localhost:3000/x", true),
            ("javascript:alert(1)", false),
            ("ftp://x", false),
            ("example.com", false),
            ("/jobs/1", false),
            ("http://", false),
        ] {
            assert_eq!(is_web_url(url), ok, "{url}");
        }
    }

    #[test]
    fn status_round_trips_through_text() {
        for status in ApplicationStatus::ALL {
            assert_eq!(
                ApplicationStatus::try_from(status.as_str().to_string()),
                Ok(status)
            );
        }
        assert!(ApplicationStatus::try_from("archived".to_string()).is_err());
    }
}
