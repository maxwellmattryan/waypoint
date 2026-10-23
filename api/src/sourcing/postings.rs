//! The read-only `job_postings` routes: `GET /postings` (filtered, newest first, paged on
//! `id`) and `GET /postings/{id}`. The list leaves out `description`, which can run to
//! 100,000 characters, so a page stays small. See `docs/sourcing.md`.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::serde::json::Json;
use serde::Serialize;
use sqlx::PgPool;

use super::Source;
use crate::applications::{ApiError, FieldError, ValidationErrors, internal};
use crate::host::LocalHost;
use crate::memories::{filter, present};

const BAD_FLAG: &str = "Must be true or false";

/// One `job_postings` row without its `description`.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct JobPosting {
    pub id: i64,
    /// The board's provider.
    pub source: Source,
    /// The board's slug, as in its board URL.
    pub board: String,
    /// The posting's id on the board.
    pub external_id: String,
    pub title: String,
    pub company: String,
    /// The posting's page on the board.
    pub url: String,
    /// Where to apply, when the board gives a separate link.
    #[schema(required = true)]
    pub apply_url: Option<String>,
    #[schema(required = true)]
    pub department: Option<String>,
    #[schema(required = true)]
    pub employment_type: Option<String>,
    /// The board's location text, as given.
    #[schema(required = true)]
    pub location_raw: Option<String>,
    /// The city parsed from the location.
    #[schema(required = true)]
    pub city: Option<String>,
    /// The region or state parsed from the location.
    #[schema(required = true)]
    pub region: Option<String>,
    /// The country parsed from the location.
    #[schema(required = true)]
    pub country: Option<String>,
    /// Whether the posting is remote.
    pub remote: bool,
    /// When the board says the posting was published or last updated.
    #[schema(required = true)]
    pub posted_at: Option<DateTime<Utc>>,
    /// When Waypoint first saw the posting.
    pub first_seen_at: DateTime<Utc>,
    /// The last successful fetch that listed the posting.
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the board stopped listing the posting; `null` while it's listed.
    #[schema(required = true)]
    pub removed_at: Option<DateTime<Utc>>,
}

/// The `GET /postings/{id}` body: the posting with its description.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct JobPostingDetail {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub posting: JobPosting,
    /// The posting's description as plain text; can be empty.
    pub description: String,
}

const POSTING_COLUMNS: &str = "id, source, board, external_id, title, company, url, apply_url, \
                               department, employment_type, location_raw, city, region, \
                               country, remote, posted_at, first_seen_at, last_seen_at, \
                               created_at, updated_at, removed_at";

/// The `GET /postings` filters, already checked; `None` means any.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostingFilters {
    /// A case-insensitive substring of `company`.
    pub company: Option<String>,
    pub source: Option<Source>,
    /// A case-insensitive substring of `location_raw`, `city`, `region` or `country`.
    pub location: Option<String>,
    pub remote: Option<bool>,
    /// Whether to include postings with `removed_at` set.
    pub include_removed: bool,
}

/// Parses an optional `true`/`false` query value, recording an error for anything else.
/// Blank counts as absent.
fn flag(errors: &mut Vec<FieldError>, field: &str, value: Option<&str>) -> Option<bool> {
    match present(value)? {
        "true" => Some(true),
        "false" => Some(false),
        _ => {
            errors.push(FieldError {
                field: field.into(),
                message: BAD_FLAG.into(),
            });
            None
        }
    }
}

/// Up to `limit` postings matching `filters` with `id < before` (any id when `None`),
/// newest first.
pub async fn list_postings(
    pool: &PgPool,
    filters: &PostingFilters,
    limit: i64,
    before: Option<i64>,
) -> Result<Vec<JobPosting>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {POSTING_COLUMNS} FROM job_postings \
         WHERE ($1::text IS NULL OR strpos(lower(company), lower($1)) > 0) \
         AND ($2::text IS NULL OR source = $2) \
         AND ($3::text IS NULL OR strpos(lower(coalesce(location_raw, '')), lower($3)) > 0 \
              OR strpos(lower(coalesce(city, '')), lower($3)) > 0 \
              OR strpos(lower(coalesce(region, '')), lower($3)) > 0 \
              OR strpos(lower(coalesce(country, '')), lower($3)) > 0) \
         AND ($4::boolean IS NULL OR remote = $4) \
         AND ($5::boolean OR removed_at IS NULL) \
         AND ($6::bigint IS NULL OR id < $6) \
         ORDER BY id DESC LIMIT $7"
    ))
    .bind(filters.company.as_deref())
    .bind(filters.source.map(Source::as_str))
    .bind(filters.location.as_deref())
    .bind(filters.remote)
    .bind(filters.include_removed)
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// One posting with its description, removed or not.
pub async fn get_posting(pool: &PgPool, id: i64) -> Result<Option<JobPostingDetail>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {POSTING_COLUMNS}, description FROM job_postings WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// `GET /postings`: sourced postings matching every given filter.
#[utoipa::path(
    get,
    path = "/postings",
    tag = "sourcing",
    operation_id = "list_postings",
    params(
        ("company" = Option<String>, Query, description = "Only postings whose company contains this text, ignoring case; blank means any"),
        ("source" = Option<Source>, Query, description = "Only postings from this board provider; blank means any"),
        ("location" = Option<String>, Query, description = "Only postings whose raw location, city, region or country contains this text, ignoring case; blank means any"),
        ("remote" = Option<bool>, Query, description = "`true` for only remote postings, `false` for only non-remote ones; blank means any"),
        ("include_removed" = Option<bool>, Query, description = "`true` also returns postings the board stopped listing; absent, blank or `false` hides them"),
        ("limit" = Option<i64>, Query, description = "Most postings to return; default 50, clamped to 1..=200"),
        ("before" = Option<i64>, Query, description = "Only postings with an `id` below this one, for the next page"),
    ),
    responses(
        (status = 200, description = "Postings without their `description`, `id` descending", body = [JobPosting]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 422, description = "`source` is not a known value, or `remote` or `include_removed` is not `true` or `false`", body = ValidationErrors),
        (status = 500, description = "The postings could not be read"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::get(
    "/postings?<company>&<source>&<location>&<remote>&<include_removed>&<limit>&<before>"
)]
pub async fn list(
    _host: LocalHost,
    company: Option<&str>,
    source: Option<&str>,
    location: Option<&str>,
    remote: Option<&str>,
    include_removed: Option<&str>,
    limit: Option<i64>,
    before: Option<i64>,
    pool: &State<PgPool>,
) -> Result<Json<Vec<JobPosting>>, ApiError> {
    let mut errors = Vec::new();
    let source: Option<Source> = filter(&mut errors, "source", source);
    let remote = flag(&mut errors, "remote", remote);
    let include_removed = flag(&mut errors, "include_removed", include_removed);
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }
    let filters = PostingFilters {
        company: present(company).map(str::to_string),
        source,
        location: present(location).map(str::to_string),
        remote,
        include_removed: include_removed.unwrap_or(false),
    };
    list_postings(pool, &filters, crate::runs::page_size(limit), before)
        .await
        .map(Json)
        .map_err(internal("list postings"))
}

/// `GET /postings/{id}`: one posting with its description.
#[utoipa::path(
    get,
    path = "/postings/{id}",
    tag = "sourcing",
    operation_id = "get_posting",
    params(("id" = i64, Path, description = "The posting's id")),
    responses(
        (status = 200, description = "The posting and its plain-text description, removed or not", body = JobPostingDetail),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No posting has this id"),
        (status = 500, description = "The posting could not be read"),
    )
)]
#[rocket::get("/postings/<id>")]
pub async fn show(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<JobPostingDetail>, ApiError> {
    get_posting(pool, id)
        .await
        .map_err(internal("read a posting"))?
        .map(Json)
        .ok_or(ApiError::NotFound)
}
