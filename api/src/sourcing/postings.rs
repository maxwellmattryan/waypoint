//! The `job_postings` routes: `GET /postings` (filtered, newest first, paged on `id`),
//! `GET /postings/{id}`, and a posting's fit scores, `GET /postings/{id}/fit` and
//! `POST /postings/{id}/fit` ([`crate::workflows::score_fit`]). The list leaves out
//! `description`, which can run to 100,000 characters, so a page stays small; each
//! posting carries its current fit score. See `docs/sourcing.md`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::serde::json::{self, Json};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;

use super::Source;
use crate::applications::{ApiError, FieldError, JsonContent, ValidationErrors, internal, parsed};
use crate::events::{Event, EventBus};
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::Wake;
use crate::memories::{filter, present};
use crate::runs;
use crate::workflows::score_fit::{self, FitBand, PostingFit, ScoreFitQueued, ScoreFitRequest};

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
    /// The posting's current (latest applied) fit score, 0 to 100; `null` when none was
    /// ever applied.
    #[schema(required = true)]
    pub fit_score: Option<i32>,
    /// The band of `fit_score`; `null` with it.
    #[schema(required = true)]
    #[sqlx(skip)]
    pub fit_band: Option<FitBand>,
    /// Whether a fit score newer than `fit_score` waits in the review queue.
    pub fit_in_review: bool,
}

impl JobPosting {
    /// Fills `fit_band` from `fit_score`.
    fn with_band(mut self) -> Self {
        self.fit_band = self.fit_score.map(FitBand::of);
        self
    }
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
                               created_at, updated_at, removed_at, \
                               fit.fit_score, EXISTS (SELECT 1 FROM posting_fit_scores s \
                                 JOIN review_items r ON r.id = s.review_item_id \
                                 AND r.status = 'pending' \
                                 WHERE s.posting_id = job_postings.id \
                                 AND s.status = 'in_review' \
                                 AND s.id > COALESCE(fit.fit_id, 0)) AS fit_in_review";

/// The `FROM` of a posting query: each posting with its latest applied fit score.
const POSTING_FROM: &str = "job_postings LEFT JOIN LATERAL (SELECT id AS fit_id, \
                            score AS fit_score FROM posting_fit_scores \
                            WHERE posting_id = job_postings.id AND status = 'applied' \
                            ORDER BY updated_at DESC, id DESC LIMIT 1) fit ON true";

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
    let found: Vec<JobPosting> = sqlx::query_as(&format!(
        "SELECT {POSTING_COLUMNS} FROM {POSTING_FROM} \
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
    .await?;
    Ok(found.into_iter().map(JobPosting::with_band).collect())
}

/// One posting with its description, removed or not.
pub async fn get_posting(pool: &PgPool, id: i64) -> Result<Option<JobPostingDetail>, sqlx::Error> {
    let found: Option<JobPostingDetail> = sqlx::query_as(&format!(
        "SELECT {POSTING_COLUMNS}, description FROM {POSTING_FROM} WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(found.map(|mut detail| {
        detail.posting = detail.posting.with_band();
        detail
    }))
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

/// `GET /postings/{id}/fit`: the posting's fit scores and latest scoring run.
#[utoipa::path(
    get,
    path = "/postings/{id}/fit",
    tag = "sourcing",
    operation_id = "get_posting_fit",
    params(("id" = i64, Path, description = "The posting's id")),
    responses(
        (status = 200, description = "The latest applied score, a newer score waiting in review, and the latest `score_fit` run", body = PostingFit),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No posting has this id"),
        (status = 500, description = "The scores could not be read"),
    )
)]
#[rocket::get("/postings/<id>/fit")]
pub async fn fit(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<PostingFit>, ApiError> {
    let failed = |err: sqlx::Error| internal("read a posting's fit")(err);
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM job_postings WHERE id = $1")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(failed)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }
    score_fit::posting_fit(pool, id)
        .await
        .map(Json)
        .map_err(failed)
}

/// `POST /postings/{id}/fit`: queues a `score_fit` run for the posting, or returns the one
/// already waiting or running.
#[utoipa::path(
    post,
    path = "/postings/{id}/fit",
    tag = "sourcing",
    operation_id = "score_posting_fit",
    params(("id" = i64, Path, description = "The posting's id")),
    request_body = ScoreFitRequest,
    responses(
        (status = 202, description = "The `score_fit` run: the one already waiting (`queued` or `paused`) or `running` with `reused: true`, or a new one (`paused` while the kill switch is on)", body = ScoreFitQueued),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No posting has this id"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 500, description = "The run could not be queued"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/postings/<id>/fit", data = "<body>")]
pub async fn score(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<ScoreFitRequest>, json::Error<'_>>,
) -> Result<(Status, Json<ScoreFitQueued>), ApiError> {
    let ScoreFitRequest {} = parsed(body)?;
    let failed = |err: sqlx::Error| internal("queue a fit score")(err);
    let mut tx = pool.begin().await.map_err(failed)?;
    let locked: Option<i64> =
        sqlx::query_scalar("SELECT id FROM job_postings WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failed)?;
    if locked.is_none() {
        return Err(ApiError::NotFound);
    }
    if let Some(run) = score_fit::active_run(&mut tx, id).await.map_err(failed)? {
        tx.commit().await.map_err(failed)?;
        return Ok((Status::Accepted, Json(ScoreFitQueued { run, reused: true })));
    }
    let run = runs::insert_run(
        &mut tx,
        bus,
        score_fit::WORKFLOW,
        &[score_fit::STEP],
        json!({"posting_id": id}),
        Duration::ZERO,
    )
    .await
    .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    bus.publish(Event::RunUpdated { run: run.clone() });
    wake.0.notify_one();
    Ok((
        Status::Accepted,
        Json(ScoreFitQueued { run, reused: false }),
    ))
}
