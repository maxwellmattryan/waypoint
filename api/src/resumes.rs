//! Resume versions: every resume PDF the user sends out, stored in Postgres with its
//! extracted text, and linked to applications through `applications.resume_version_id`.
//!
//! An upload is the raw PDF as the body (`Content-Type: application/pdf`), not multipart,
//! so [`NotCrossSite`] guards it like any other changing route. Text extraction runs
//! during the request; a PDF it can't read (encrypted, scanned, malformed) is still
//! stored, with `text: null`. Versions are never edited: upload a new one instead.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::data::{Data, ToByteUnit};
use rocket::http::{ContentType, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::{self, Responder, Response};
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::applications::{
    ApiError, FieldError, ValidationErrors, internal, is_foreign_key_violation, text,
};
use crate::host::{LocalHost, NotCrossSite};

/// The largest PDF an upload accepts: 10 MiB.
pub const MAX_PDF_BYTES: usize = 10 * 1024 * 1024;
/// Most characters in `name`.
pub const MAX_NAME: usize = 200;
/// Most characters in `filename`.
pub const MAX_FILENAME: usize = 255;
/// The stored filename when the upload gives none.
pub const DEFAULT_FILENAME: &str = "resume.pdf";

/// Every PDF starts with this.
const PDF_MAGIC: &[u8] = b"%PDF-";

/// One resume version, without its text or bytes: the `GET /resumes` item.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ResumeVersion {
    pub id: i64,
    /// The user's label, e.g. "Backend, Oct 2026".
    pub name: String,
    /// The uploaded file's original name.
    pub filename: String,
    pub size_bytes: i32,
    /// SHA-256 of the PDF bytes, lowercase hex.
    pub sha256: String,
    /// Whether text could be extracted from the PDF.
    pub has_text: bool,
    /// How many applications reference this version.
    pub application_count: i64,
    pub created_at: DateTime<Utc>,
}

/// One resume version with its extracted text: the `GET /resumes/{id}` body.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ResumeVersionDetail {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub version: ResumeVersion,
    /// The extracted text, or `null` when none could be extracted.
    #[schema(required = true)]
    pub text: Option<String>,
}

/// The `PUT /applications/{id}/resume` body. `null` clears the link.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct ResumeLink {
    // `deserialize_with` makes the key required: a missing key is a 400, not a clear.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schema(required = true)]
    pub resume_version_id: Option<i64>,
}

// Documents a raw PDF body as a binary string rather than utoipa's array of integers.
/// Raw PDF bytes.
#[derive(utoipa::ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct PdfBytes(pub Vec<u8>);

/// The list columns, for `resume_versions AS r`.
const COLUMNS: &str = "r.id, r.name, r.filename, r.size_bytes, r.sha256, \
                       r.text IS NOT NULL AS has_text, \
                       (SELECT count(*) FROM applications a WHERE a.resume_version_id = r.id) \
                       AS application_count, r.created_at";

/// Admits only a `Content-Type: application/pdf` body. Taken after [`NotCrossSite`], so
/// a form or multipart body is a 403 and any other type (or none) is a 415.
pub struct PdfContent;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for PdfContent {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        if req.content_type().is_some_and(|ct| ct.is_pdf()) {
            Outcome::Success(Self)
        } else {
            Outcome::Error((Status::UnsupportedMediaType, ()))
        }
    }
}

/// Extracts a PDF's text, or `None` when the extractor fails or panics. The text is
/// trimmed, NUL characters (which Postgres `TEXT` rejects) are removed, and an empty
/// result is `None`.
fn extract_text(bytes: &[u8]) -> Option<String> {
    let text = match catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes)
    })) {
        Ok(Ok(text)) => text,
        Ok(Err(err)) => {
            rocket::warn!("resumes: cannot extract text: {err}");
            return None;
        }
        Err(_) => {
            rocket::warn!("resumes: the text extractor panicked");
            return None;
        }
    };
    normalise(&text)
}

/// Removes NUL characters, trims, and turns an empty result into `None`.
fn normalise(text: &str) -> Option<String> {
    let text = text.replace('\0', "");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Runs [`extract_text`] off the async runtime.
async fn extract(bytes: Arc<Vec<u8>>) -> Option<String> {
    match tokio::task::spawn_blocking(move || extract_text(&bytes)).await {
        Ok(text) => text,
        Err(err) => {
            rocket::warn!("resumes: the text extraction task failed: {err}");
            None
        }
    }
}

/// The `Content-Disposition` filename: ASCII letters, digits, space, `.`, `-` and `_`
/// are kept and everything else becomes `_`, so the header can't be broken out of.
fn safe_filename(filename: &str) -> String {
    let safe: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.trim().is_empty() {
        DEFAULT_FILENAME.to_string()
    } else {
        safe
    }
}

/// Validated upload input, ready to store.
struct Upload {
    name: String,
    filename: String,
}

fn validate(
    name: Option<&str>,
    filename: Option<&str>,
    body: &[u8],
) -> Result<Upload, ValidationErrors> {
    let mut errors = Vec::new();
    let name = text(&mut errors, "name", name, MAX_NAME);
    if name.is_none() {
        errors.push(FieldError {
            field: "name".into(),
            message: "Required".into(),
        });
    }
    let filename = text(&mut errors, "filename", filename, MAX_FILENAME);
    if body.is_empty() {
        errors.push(FieldError {
            field: "file".into(),
            message: "Required".into(),
        });
    } else if !body.starts_with(PDF_MAGIC) {
        errors.push(FieldError {
            field: "file".into(),
            message: "Must be a PDF".into(),
        });
    }
    if errors.is_empty() {
        Ok(Upload {
            name: name.unwrap_or_default(),
            filename: filename.unwrap_or_else(|| DEFAULT_FILENAME.to_string()),
        })
    } else {
        Err(ValidationErrors { errors })
    }
}

/// Every resume version, the newest first.
#[utoipa::path(
    get,
    path = "/resumes",
    tag = "resumes",
    operation_id = "list_resumes",
    responses(
        (status = 200, description = "Every resume version, newest `created_at` first, ties by `id` descending", body = [ResumeVersion]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The resume versions could not be read"),
    )
)]
#[rocket::get("/resumes")]
pub async fn list(
    _host: LocalHost,
    pool: &State<PgPool>,
) -> Result<Json<Vec<ResumeVersion>>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM resume_versions AS r ORDER BY r.created_at DESC, r.id DESC"
    ))
    .fetch_all(pool.inner())
    .await
    .map(Json)
    .map_err(internal("list resume versions"))
}

/// Uploads a resume PDF as a new version and extracts its text.
#[utoipa::path(
    post,
    path = "/resumes",
    tag = "resumes",
    operation_id = "upload_resume",
    params(
        ("name" = String, Query, description = "The version's label: required, 1–200 characters after trimming"),
        ("filename" = Option<String>, Query, description = "The original file name, at most 255 characters; `resume.pdf` when blank or missing"),
    ),
    request_body(content = inline(PdfBytes), content_type = "application/pdf", description = "The raw PDF bytes, 1 byte to 10 MiB, starting with `%PDF-`"),
    responses(
        (status = 201, description = "The stored version (`has_text` is false when no text could be extracted)", body = ResumeVersion),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 413, description = "The body is over 10 MiB"),
        (status = 415, description = "The body is not `application/pdf`"),
        (status = 422, description = "`name`, `filename` or `file` is invalid", body = ValidationErrors),
        (status = 500, description = "The version could not be stored"),
    )
)]
#[rocket::post("/resumes?<name>&<filename>", data = "<data>")]
#[allow(clippy::too_many_arguments)]
pub async fn upload(
    _host: LocalHost,
    _site: NotCrossSite,
    _pdf: PdfContent,
    name: Option<&str>,
    filename: Option<&str>,
    pool: &State<PgPool>,
    data: Data<'_>,
) -> Result<(Status, Json<ResumeVersion>), ApiError> {
    // An explicit `open` limit replaces Rocket's default `limits` for this body.
    let body = data
        .open((MAX_PDF_BYTES + 1).bytes())
        .into_bytes()
        .await
        .map_err(|err| {
            rocket::warn!("resumes: cannot read the upload: {}", err.kind());
            ApiError::BadRequest
        })?;
    if !body.is_complete() || body.len() > MAX_PDF_BYTES {
        return Err(ApiError::TooLarge);
    }
    let bytes = body.into_inner();
    let upload = validate(name, filename, &bytes).map_err(ApiError::Invalid)?;

    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let size_bytes = i32::try_from(bytes.len()).map_err(|_| ApiError::TooLarge)?;
    let bytes = Arc::new(bytes);
    let text = extract(Arc::clone(&bytes)).await;

    let version: ResumeVersion = sqlx::query_as(&format!(
        "INSERT INTO resume_versions AS r (name, filename, size_bytes, sha256, pdf, text) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING {COLUMNS}"
    ))
    .bind(upload.name)
    .bind(upload.filename)
    .bind(size_bytes)
    .bind(sha256)
    .bind(bytes.as_slice())
    .bind(text)
    .fetch_one(pool.inner())
    .await
    .map_err(internal("store a resume version"))?;
    Ok((Status::Created, Json(version)))
}

/// One resume version with its text.
#[utoipa::path(
    get,
    path = "/resumes/{id}",
    tag = "resumes",
    operation_id = "get_resume",
    params(("id" = i64, Path, description = "The resume version's id")),
    responses(
        (status = 200, description = "The resume version with its extracted text", body = ResumeVersionDetail),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No resume version has this id"),
        (status = 500, description = "The resume version could not be read"),
    )
)]
#[rocket::get("/resumes/<id>")]
pub async fn show(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<ResumeVersionDetail>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS}, r.text FROM resume_versions AS r WHERE r.id = $1"
    ))
    .bind(id)
    .fetch_optional(pool.inner())
    .await
    .map_err(internal("read a resume version"))?
    .map(Json)
    .ok_or(ApiError::NotFound)
}

/// A stored PDF, served inline with headers that keep the browser from sniffing or
/// caching it.
pub struct PdfFile {
    bytes: Vec<u8>,
    filename: String,
}

impl<'r> Responder<'r, 'static> for PdfFile {
    fn respond_to(self, _req: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(ContentType::PDF)
            .raw_header(
                "Content-Disposition",
                format!("inline; filename=\"{}\"", safe_filename(&self.filename)),
            )
            .raw_header("X-Content-Type-Options", "nosniff")
            .raw_header("Cache-Control", "no-store")
            .sized_body(self.bytes.len(), Cursor::new(self.bytes))
            .ok()
    }
}

/// The exact PDF bytes of a resume version.
#[utoipa::path(
    get,
    path = "/resumes/{id}/pdf",
    tag = "resumes",
    operation_id = "get_resume_pdf",
    params(("id" = i64, Path, description = "The resume version's id")),
    responses(
        (status = 200, description = "The stored PDF, `inline` with `nosniff` and `no-store`", content_type = "application/pdf", body = inline(PdfBytes)),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No resume version has this id"),
        (status = 500, description = "The PDF could not be read"),
    )
)]
#[rocket::get("/resumes/<id>/pdf")]
pub async fn pdf(_host: LocalHost, id: i64, pool: &State<PgPool>) -> Result<PdfFile, ApiError> {
    let row: Option<(Vec<u8>, String)> =
        sqlx::query_as("SELECT pdf, filename FROM resume_versions WHERE id = $1")
            .bind(id)
            .fetch_optional(pool.inner())
            .await
            .map_err(internal("read a resume PDF"))?;
    let (bytes, filename) = row.ok_or(ApiError::NotFound)?;
    Ok(PdfFile { bytes, filename })
}

/// Deletes a resume version permanently, unless an application references it.
#[utoipa::path(
    delete,
    path = "/resumes/{id}",
    tag = "resumes",
    operation_id = "delete_resume",
    params(("id" = i64, Path, description = "The resume version's id")),
    responses(
        (status = 204, description = "The resume version is deleted"),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No resume version has this id"),
        (status = 409, description = "An application references this version; unlink it first"),
        (status = 500, description = "The resume version could not be deleted"),
    )
)]
#[rocket::delete("/resumes/<id>")]
pub async fn delete(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Status, ApiError> {
    // The foreign key is `ON DELETE RESTRICT`, so a linked version fails here atomically.
    let done = sqlx::query("DELETE FROM resume_versions WHERE id = $1")
        .bind(id)
        .execute(pool.inner())
        .await
        .map_err(|err| {
            if is_foreign_key_violation(&err) {
                ApiError::Conflict
            } else {
                internal("delete a resume version")(err)
            }
        })?;
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
    fn safe_filename_keeps_the_header_intact() {
        for (input, want) in [
            ("resume.pdf", "resume.pdf"),
            ("My Resume - v2_final.pdf", "My Resume - v2_final.pdf"),
            ("a\"b.pdf", "a_b.pdf"),
            ("x\r\nSet-Cookie: y.pdf", "x__Set-Cookie_ y.pdf"),
            ("résumé.pdf", "r_sum_.pdf"),
            ("履歴書.pdf", "___.pdf"),
            ("../../etc/passwd", ".._.._etc_passwd"),
            ("", "resume.pdf"),
            ("   ", "resume.pdf"),
        ] {
            assert_eq!(safe_filename(input), want, "{input:?}");
        }
    }

    #[test]
    fn normalise_trims_strips_nul_and_nulls_empty() {
        for (input, want) in [
            ("  Hello\n", Some("Hello")),
            ("a\0b", Some("ab")),
            ("\0 \n\t", None),
            ("", None),
        ] {
            assert_eq!(normalise(input).as_deref(), want, "{input:?}");
        }
    }
}
