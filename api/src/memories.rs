//! Memories: what Waypoint knows about the user. One `memories` row per fact, preference
//! or answer-bank entry, each with its provenance (where it came from) and whether the
//! user has confirmed it. The routes let the user list, search, add, edit, confirm and
//! delete them; later workflows add memories in-process through [`insert`].
//!
//! The HTTP routes never set `status` or provenance: a memory created over HTTP is
//! `confirmed` and `manual`, and an edit keeps both. Only [`insert`] stores a `proposed`
//! or sourced memory, so a client can't forge where a memory came from.
//!
//! Input is normalised here and nowhere else: content is trimmed, tags are normalised by
//! [`normalise_tags`], and the topic must belong to the kind ([`MemoryKind::topics`]). A
//! body that doesn't parse (or names an unknown kind or topic) is a 400; a body that parses
//! but breaks a rule is a 422 naming every failing field.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, PgPool};

use crate::applications::{
    ApiError, FieldError, JsonContent, REQUIRED, ValidationErrors, internal, parsed, text,
};
use crate::host::{LocalHost, NotCrossSite};

/// Most characters in `content`.
pub const MAX_CONTENT: usize = 5_000;
/// Most tags on one memory.
pub const MAX_TAGS: usize = 20;
/// Most characters in one tag.
pub const MAX_TAG: usize = 50;

const WRONG_TOPIC: &str = "Not a topic for this kind";

/// A closed set of values stored as TEXT: the wire and database names, `ALL`, parsing
/// from text and decoding straight from a TEXT column, in the shape of
/// [`crate::applications::ApplicationStatus`]. Shared with [`crate::memory_signals`].
macro_rules! text_enum {
    ($name:ident, $what:literal, [$($variant:ident => $text:literal),+ $(,)?]) => {
        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The wire and database name.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, String> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|v| v.as_str() == value)
                    .ok_or_else(|| format!(concat!("unknown ", $what, " {:?}"), value))
            }
        }

        impl ::sqlx::Type<::sqlx::Postgres> for $name {
            fn type_info() -> ::sqlx::postgres::PgTypeInfo {
                <String as ::sqlx::Type<::sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &::sqlx::postgres::PgTypeInfo) -> bool {
                <String as ::sqlx::Type<::sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'r> ::sqlx::Decode<'r, ::sqlx::Postgres> for $name {
            fn decode(
                value: ::sqlx::postgres::PgValueRef<'r>,
            ) -> Result<Self, ::sqlx::error::BoxDynError> {
                let text = <String as ::sqlx::Decode<::sqlx::Postgres>>::decode(value)?;
                Ok(Self::try_from(text)?)
            }
        }
    };
}
pub(crate) use text_enum;

/// What sort of thing a memory is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Something true about the user: a position, a skill, a degree.
    Fact,
    /// What the user wants from a role.
    Preference,
    /// A ready answer to a question applications ask.
    Answer,
}

text_enum!(MemoryKind, "memory kind", [
    Fact => "fact",
    Preference => "preference",
    Answer => "answer",
]);

impl MemoryKind {
    /// The topics a memory of this kind may have, in display order. The single Rust source
    /// of the kind/topic table; the `memories_topic_check` constraint mirrors it.
    pub fn topics(self) -> &'static [MemoryTopic] {
        use MemoryTopic::*;
        match self {
            Self::Fact => &[Position, Skill, Education, Other],
            Self::Preference => &[
                TargetRole,
                Seniority,
                Location,
                CompFloor,
                Industry,
                DealBreaker,
                Other,
            ],
            Self::Answer => &[
                WorkAuthorization,
                NoticePeriod,
                SalaryExpectation,
                WhyLeaving,
                Other,
            ],
        }
    }
}

/// What a memory is about. Which topics are allowed depends on the kind
/// ([`MemoryKind::topics`]); `other` is allowed for every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTopic {
    Position,
    Skill,
    Education,
    TargetRole,
    Seniority,
    /// Where the user will work, remote included.
    Location,
    CompFloor,
    Industry,
    DealBreaker,
    WorkAuthorization,
    NoticePeriod,
    SalaryExpectation,
    WhyLeaving,
    Other,
}

text_enum!(MemoryTopic, "memory topic", [
    Position => "position",
    Skill => "skill",
    Education => "education",
    TargetRole => "target_role",
    Seniority => "seniority",
    Location => "location",
    CompFloor => "comp_floor",
    Industry => "industry",
    DealBreaker => "deal_breaker",
    WorkAuthorization => "work_authorization",
    NoticePeriod => "notice_period",
    SalaryExpectation => "salary_expectation",
    WhyLeaving => "why_leaving",
    Other => "other",
]);

/// Whether the user has confirmed a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    /// Learned by Waypoint, waiting for the user.
    Proposed,
    Confirmed,
}

text_enum!(MemoryStatus, "memory status", [
    Proposed => "proposed",
    Confirmed => "confirmed",
]);

/// Where a memory came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Added by the user.
    Manual,
    /// Read from a resume version; `source_ref` is its id.
    Resume,
    /// Read from a LinkedIn export.
    Linkedin,
    /// Learned in an agent run; `source_ref` is its id.
    Run,
    /// Read from an email; `source_ref` is its message id.
    Email,
}

text_enum!(SourceKind, "source kind", [
    Manual => "manual",
    Resume => "resume",
    Linkedin => "linkedin",
    Run => "run",
    Email => "email",
]);

/// One `memories` row: the body of every route that returns a memory.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Memory {
    pub id: i64,
    pub kind: MemoryKind,
    pub topic: MemoryTopic,
    pub content: String,
    /// Normalised: lowercase, single-spaced, no blanks or duplicates.
    pub tags: Vec<String>,
    pub status: MemoryStatus,
    // The `Option` fields are always serialized (as `null` when empty), so they are
    // required in the schema.
    /// When the memory became confirmed; `null` while proposed.
    #[schema(required = true)]
    pub confirmed_at: Option<DateTime<Utc>>,
    pub source_kind: SourceKind,
    /// An id in the source as text: a resume version id, a run id, a message id.
    #[schema(required = true)]
    pub source_ref: Option<String>,
    /// A human label for the source, e.g. "Resume · Staff Eng v3".
    #[schema(required = true)]
    pub source_label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// `search` is never selected.
const COLUMNS: &str = "id, kind, topic, content, tags, status, confirmed_at, source_kind, \
                       source_ref, source_label, created_at, updated_at";

/// The `POST /memories` and `PUT /memories/{id}` body. Unknown keys, `status` and the
/// provenance fields included, are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
pub struct MemoryInput {
    pub kind: MemoryKind,
    /// Must be one of the kind's topics.
    pub topic: MemoryTopic,
    /// Required; a missing or blank value is a 422. At most 5,000 characters.
    #[serde(default)]
    #[schema(required = true)]
    pub content: String,
    /// At most 20 tags of at most 50 characters each, normalised on write.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Validated input, ready to store. Only [`validate`] makes one.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryFields {
    kind: MemoryKind,
    topic: MemoryTopic,
    content: String,
    tags: Vec<String>,
}

/// Trims a tag, lowercases it and collapses each run of whitespace to one space.
pub fn normalise_tag(tag: &str) -> String {
    tag.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Normalises every tag ([`normalise_tag`]), drops blanks and drops duplicates, keeping
/// the first occurrence and the order otherwise.
pub fn normalise_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = normalise_tag(tag);
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Checks a body against every rule at once.
pub fn validate(input: &MemoryInput) -> Result<MemoryFields, ValidationErrors> {
    let mut errors = Vec::new();
    let content = text(
        &mut errors,
        "content",
        Some(input.content.as_str()),
        MAX_CONTENT,
    );
    if content.is_none() {
        errors.push(FieldError {
            field: "content".into(),
            message: REQUIRED.into(),
        });
    }
    if !input.kind.topics().contains(&input.topic) {
        errors.push(FieldError {
            field: "topic".into(),
            message: WRONG_TOPIC.into(),
        });
    }
    let tags = normalise_tags(&input.tags);
    if tags.len() > MAX_TAGS {
        errors.push(FieldError {
            field: "tags".into(),
            message: format!("At most {MAX_TAGS} tags"),
        });
    }
    if tags.iter().any(|tag| tag.chars().count() > MAX_TAG) {
        errors.push(FieldError {
            field: "tags".into(),
            message: format!("Each tag at most {MAX_TAG} characters"),
        });
    }

    if errors.is_empty() {
        Ok(MemoryFields {
            kind: input.kind,
            topic: input.topic,
            content: content.unwrap_or_default(),
            tags,
        })
    } else {
        Err(ValidationErrors { errors })
    }
}

/// Where a stored memory came from.
#[derive(Debug, Clone, PartialEq)]
pub struct MemorySource {
    pub kind: SourceKind,
    /// An id in the source, as text.
    pub reference: Option<String>,
    /// A human label for the source.
    pub label: Option<String>,
}

impl MemorySource {
    /// Added by the user: no reference or label.
    pub fn manual() -> Self {
        Self {
            kind: SourceKind::Manual,
            reference: None,
            label: None,
        }
    }
}

/// A memory to store with [`insert`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewMemory {
    pub fields: MemoryFields,
    pub status: MemoryStatus,
    pub source: MemorySource,
}

/// Stores a memory, setting `confirmed_at` to now when it is stored confirmed. The one
/// way to store a `proposed` or sourced memory.
pub async fn insert(executor: impl PgExecutor<'_>, memory: &NewMemory) -> sqlx::Result<Memory> {
    let fields = &memory.fields;
    sqlx::query_as(&format!(
        "INSERT INTO memories (kind, topic, content, tags, status, confirmed_at, \
         source_kind, source_ref, source_label) \
         VALUES ($1, $2, $3, $4, $5::text, \
         CASE WHEN $5::text = 'confirmed' THEN now() END, $6, $7, $8) \
         RETURNING {COLUMNS}"
    ))
    .bind(fields.kind.as_str())
    .bind(fields.topic.as_str())
    .bind(&fields.content)
    .bind(&fields.tags)
    .bind(memory.status.as_str())
    .bind(memory.source.kind.as_str())
    .bind(&memory.source.reference)
    .bind(&memory.source.label)
    .fetch_one(executor)
    .await
}

/// A query parameter, trimmed, with a blank one treated as absent.
pub(crate) fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// Parses an optional filter value, recording an `Unknown <field>` error for a bad one.
pub(crate) fn filter<T: TryFrom<String>>(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: Option<&str>,
) -> Option<T> {
    let value = present(value)?;
    let parsed = T::try_from(value.to_string()).ok();
    if parsed.is_none() {
        errors.push(FieldError {
            field: field.into(),
            message: format!("Unknown {field}"),
        });
    }
    parsed
}

/// Memories matching every given filter.
#[utoipa::path(
    get,
    path = "/memories",
    tag = "memories",
    operation_id = "list_memories",
    params(
        ("kind" = Option<MemoryKind>, Query, description = "Only memories of this kind; blank means any"),
        ("status" = Option<MemoryStatus>, Query, description = "Only memories with this status; blank means any"),
        ("tag" = Option<String>, Query, description = "Only memories carrying this tag, normalised like stored tags; blank means any"),
        ("q" = Option<String>, Query, description = "Full-text search over content, topic and tags (`websearch_to_tsquery('english', q)`); blank means no search"),
    ),
    responses(
        (status = 200, description = "Matching memories: with `q`, best match first, then newest `updated_at` first, ties by `id` descending; without `q`, newest `updated_at` first", body = [Memory]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 422, description = "`kind` or `status` is not a known value", body = ValidationErrors),
        (status = 500, description = "The memories could not be read"),
    )
)]
#[rocket::get("/memories?<kind>&<status>&<tag>&<q>")]
pub async fn list(
    _host: LocalHost,
    kind: Option<&str>,
    status: Option<&str>,
    tag: Option<&str>,
    q: Option<&str>,
    pool: &State<PgPool>,
) -> Result<Json<Vec<Memory>>, ApiError> {
    let mut errors = Vec::new();
    let kind: Option<MemoryKind> = filter(&mut errors, "kind", kind);
    let status: Option<MemoryStatus> = filter(&mut errors, "status", status);
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }
    let tag = present(tag)
        .map(normalise_tag)
        .filter(|tag| !tag.is_empty());
    let q = present(q);
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM memories \
         WHERE ($1::text IS NULL OR kind = $1) \
         AND ($2::text IS NULL OR status = $2) \
         AND ($3::text IS NULL OR tags @> ARRAY[$3::text]) \
         AND ($4::text IS NULL OR search @@ websearch_to_tsquery('english', $4)) \
         ORDER BY CASE WHEN $4::text IS NULL THEN 0 \
         ELSE ts_rank(search, websearch_to_tsquery('english', $4)) END DESC, \
         updated_at DESC, id DESC"
    ))
    .bind(kind.map(MemoryKind::as_str))
    .bind(status.map(MemoryStatus::as_str))
    .bind(tag)
    .bind(q)
    .fetch_all(pool.inner())
    .await
    .map(Json)
    .map_err(internal("list memories"))
}

/// Adds a memory the user wrote: confirmed now, source `manual`.
#[utoipa::path(
    post,
    path = "/memories",
    tag = "memories",
    operation_id = "create_memory",
    request_body = MemoryInput,
    responses(
        (status = 201, description = "The stored memory, confirmed, with source `manual`", body = Memory),
        (status = 400, description = "The body is not valid JSON for this route, or `kind` or `topic` is unknown", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "One or more fields are invalid", body = ValidationErrors),
        (status = 500, description = "The memory could not be stored"),
    )
)]
#[rocket::post("/memories", data = "<body>")]
pub async fn create(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    pool: &State<PgPool>,
    body: Result<Json<MemoryInput>, json::Error<'_>>,
) -> Result<(Status, Json<Memory>), ApiError> {
    let fields = validate(&parsed(body)?).map_err(ApiError::Invalid)?;
    let memory = NewMemory {
        fields,
        status: MemoryStatus::Confirmed,
        source: MemorySource::manual(),
    };
    let memory = insert(pool.inner(), &memory)
        .await
        .map_err(internal("store a memory"))?;
    Ok((Status::Created, Json(memory)))
}

/// One memory.
#[utoipa::path(
    get,
    path = "/memories/{id}",
    tag = "memories",
    operation_id = "get_memory",
    params(("id" = i64, Path, description = "The memory's id")),
    responses(
        (status = 200, description = "The memory", body = Memory),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No memory has this id"),
        (status = 500, description = "The memory could not be read"),
    )
)]
#[rocket::get("/memories/<id>")]
pub async fn show(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<Memory>, ApiError> {
    find(pool, id).await.map(Json)
}

/// Memory `id`, or a 404.
async fn find(pool: &PgPool, id: i64) -> Result<Memory, ApiError> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM memories WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(internal("read a memory"))?
        .ok_or(ApiError::NotFound)
}

/// Replaces a memory's kind, topic, content and tags. Its status, `confirmed_at` and
/// provenance are kept.
#[utoipa::path(
    put,
    path = "/memories/{id}",
    tag = "memories",
    operation_id = "update_memory",
    params(("id" = i64, Path, description = "The memory's id")),
    request_body = MemoryInput,
    responses(
        (status = 200, description = "The updated memory", body = Memory),
        (status = 400, description = "The body is not valid JSON for this route, or `kind` or `topic` is unknown", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No memory has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "One or more fields are invalid", body = ValidationErrors),
        (status = 500, description = "The memory could not be stored"),
    )
)]
#[rocket::put("/memories/<id>", data = "<body>")]
pub async fn update(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    body: Result<Json<MemoryInput>, json::Error<'_>>,
) -> Result<Json<Memory>, ApiError> {
    let fields = validate(&parsed(body)?).map_err(ApiError::Invalid)?;
    sqlx::query_as(&format!(
        "UPDATE memories SET kind = $2, topic = $3, content = $4, tags = $5, \
         updated_at = now() WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(fields.kind.as_str())
    .bind(fields.topic.as_str())
    .bind(fields.content)
    .bind(fields.tags)
    .fetch_optional(pool.inner())
    .await
    .map_err(internal("update a memory"))?
    .map(Json)
    .ok_or(ApiError::NotFound)
}

/// Confirms a proposed memory. An already confirmed memory is returned unchanged.
#[utoipa::path(
    post,
    path = "/memories/{id}/confirm",
    tag = "memories",
    operation_id = "confirm_memory",
    params(("id" = i64, Path, description = "The memory's id")),
    responses(
        (status = 200, description = "The confirmed memory", body = Memory),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No memory has this id"),
        (status = 500, description = "The memory could not be confirmed"),
    )
)]
#[rocket::post("/memories/<id>/confirm")]
pub async fn confirm(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<Memory>, ApiError> {
    let confirmed: Option<Memory> = sqlx::query_as(&format!(
        "UPDATE memories SET status = 'confirmed', confirmed_at = now(), updated_at = now() \
         WHERE id = $1 AND status = 'proposed' RETURNING {COLUMNS}"
    ))
    .bind(id)
    .fetch_optional(pool.inner())
    .await
    .map_err(internal("confirm a memory"))?;
    match confirmed {
        Some(memory) => Ok(Json(memory)),
        // Already confirmed (returned unchanged) or missing (404).
        None => find(pool, id).await.map(Json),
    }
}

/// Deletes a memory permanently.
#[utoipa::path(
    delete,
    path = "/memories/{id}",
    tag = "memories",
    operation_id = "delete_memory",
    params(("id" = i64, Path, description = "The memory's id")),
    responses(
        (status = 204, description = "The memory is deleted"),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No memory has this id"),
        (status = 500, description = "The memory could not be deleted"),
    )
)]
#[rocket::delete("/memories/<id>")]
pub async fn delete(
    _host: LocalHost,
    _site: NotCrossSite,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Status, ApiError> {
    let done = sqlx::query("DELETE FROM memories WHERE id = $1")
        .bind(id)
        .execute(pool.inner())
        .await
        .map_err(internal("delete a memory"))?;
    if done.rows_affected() == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(Status::NoContent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn normalise_tags() {
        assert_eq!(
            super::normalise_tags(&strings(&[
                "  Remote ",
                "",
                "   ",
                "Rust\t\n Lang",
                "remote",
                "REMOTE",
                "go",
                "rust lang",
            ])),
            strings(&["remote", "rust lang", "go"])
        );
        assert!(super::normalise_tags(&[]).is_empty());
    }

    #[test]
    fn every_topic_belongs_to_a_kind() {
        for topic in MemoryTopic::ALL {
            assert!(
                MemoryKind::ALL
                    .iter()
                    .any(|kind| kind.topics().contains(topic)),
                "{topic:?}"
            );
        }
        for kind in MemoryKind::ALL {
            assert!(kind.topics().contains(&MemoryTopic::Other), "{kind:?}");
        }
    }

    #[test]
    fn values_round_trip_through_text() {
        for topic in MemoryTopic::ALL {
            assert_eq!(
                MemoryTopic::try_from(topic.as_str().to_string()),
                Ok(*topic)
            );
        }
        for source in SourceKind::ALL {
            assert_eq!(
                SourceKind::try_from(source.as_str().to_string()),
                Ok(*source)
            );
        }
        assert!(MemoryStatus::try_from("archived".to_string()).is_err());
    }
}
