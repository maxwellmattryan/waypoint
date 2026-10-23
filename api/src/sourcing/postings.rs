//! The `job_postings` routes: `GET /postings` (filtered, newest first, paged on `id`),
//! `GET /postings/{id}`, and a posting's fit scores, `GET /postings/{id}/fit` and
//! `POST /postings/{id}/fit` ([`crate::workflows::score_fit`]), and its application
//! packets, `GET /postings/{id}/packets`, `GET /packets/{id}` and
//! `POST /postings/{id}/packet` ([`crate::workflows::prep_packet`]), and the packet's
//! `POST /packets/{id}/approval` and `POST /packets/{id}/applied`
//! ([`crate::workflows::packet_actions`]). The list leaves out
//! `description`, which can run to 100,000 characters, so a page stays small; each
//! posting carries its current fit score. See `docs/sourcing.md`.

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::serde::json::{self, Json};
use serde::Serialize;
use sqlx::PgPool;

use super::Source;
use crate::applications::{ApiError, FieldError, JsonContent, ValidationErrors, internal, parsed};
use crate::events::{Event, EventBus};
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::Wake;
use crate::memories::{filter, present};
use crate::workflows::packet_actions::{
    self, MarkAppliedRequest, PacketActionError, RequestApprovalRequest,
};
use crate::workflows::prep_packet::{
    self, ApplicationPacket, PacketQueued, PostingPackets, PreparePacketRequest, QueueError,
};
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
    let (run, reused) = score_fit::request(pool, bus, wake, id)
        .await
        .map_err(internal("queue a fit score"))?
        .ok_or(ApiError::NotFound)?;
    Ok((Status::Accepted, Json(ScoreFitQueued { run, reused })))
}

/// `GET /postings/{id}/packets`: the posting's application packets and latest prep run.
#[utoipa::path(
    get,
    path = "/postings/{id}/packets",
    tag = "sourcing",
    operation_id = "list_posting_packets",
    params(("id" = i64, Path, description = "The posting's id")),
    responses(
        (status = 200, description = "Whether the posting has an applied fit score, up to 20 packets (`id` descending) and the latest `prep_packet` run. A citation's `memory_id` is `null` once its memory is deleted", body = PostingPackets),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No posting has this id"),
        (status = 500, description = "The packets could not be read"),
    )
)]
#[rocket::get("/postings/<id>/packets")]
pub async fn packets(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<PostingPackets>, ApiError> {
    let failed = |err: sqlx::Error| internal("read a posting's packets")(err);
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM job_postings WHERE id = $1")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(failed)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }
    prep_packet::posting_packets(pool, id)
        .await
        .map(Json)
        .map_err(failed)
}

/// `GET /packets/{id}`: one application packet.
#[utoipa::path(
    get,
    path = "/packets/{id}",
    tag = "sourcing",
    operation_id = "get_packet",
    params(("id" = i64, Path, description = "The packet's id")),
    responses(
        (status = 200, description = "The packet with its posting's company and title. A citation's `memory_id` is `null` once its memory is deleted, and `resume_version_id` once the resume is", body = ApplicationPacket),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No packet has this id"),
        (status = 500, description = "The packet could not be read"),
    )
)]
#[rocket::get("/packets/<id>")]
pub async fn show_packet(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<ApplicationPacket>, ApiError> {
    prep_packet::get_packet(pool, id)
        .await
        .map_err(internal("read a packet"))?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// `POST /postings/{id}/packet`: queues a `prep_packet` run for the posting, or returns the
/// one already waiting or running.
#[utoipa::path(
    post,
    path = "/postings/{id}/packet",
    tag = "sourcing",
    operation_id = "prepare_posting_packet",
    params(("id" = i64, Path, description = "The posting's id")),
    request_body = PreparePacketRequest,
    responses(
        (status = 202, description = "The `prep_packet` run: the one already waiting (`queued` or `paused`) or `running` with `reused: true`, or a new one (`paused` while the kill switch is on)", body = PacketQueued),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No posting has this id"),
        (status = 409, description = "The posting has no applied fit score yet; nothing was queued"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 500, description = "The run could not be queued"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/postings/<id>/packet", data = "<body>")]
pub async fn prepare_packet(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<PreparePacketRequest>, json::Error<'_>>,
) -> Result<(Status, Json<PacketQueued>), ApiError> {
    let PreparePacketRequest {} = parsed(body)?;
    let failed = |err: sqlx::Error| internal("queue a packet")(err);
    let mut tx = pool.begin().await.map_err(failed)?;
    let queued = match prep_packet::queue(&mut tx, bus, id, None).await {
        Ok(queued) => queued,
        Err(QueueError::NotFound) => return Err(ApiError::NotFound),
        Err(QueueError::NotScored) => return Err(ApiError::Conflict),
        Err(QueueError::Database(err)) => return Err(failed(err)),
    };
    tx.commit().await.map_err(failed)?;
    if !queued.reused {
        bus.publish(Event::RunUpdated {
            run: queued.run.clone(),
        });
        wake.0.notify_one();
    }
    Ok((Status::Accepted, Json(queued)))
}

/// Commits `queued` (or maps its error), then publishes a new run and wakes the worker.
async fn packet_queued(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    bus: &EventBus,
    wake: &Wake,
    queued: Result<PacketQueued, PacketActionError>,
    what: &'static str,
) -> Result<(Status, Json<PacketQueued>), ApiError> {
    let queued = match queued {
        Ok(queued) => queued,
        Err(PacketActionError::NotFound) => return Err(ApiError::NotFound),
        Err(PacketActionError::NotReady | PacketActionError::NotApproved) => {
            return Err(ApiError::Conflict);
        }
        Err(PacketActionError::Database(err)) => return Err(internal(what)(err)),
    };
    tx.commit().await.map_err(internal(what))?;
    if !queued.reused {
        bus.publish(Event::RunUpdated {
            run: queued.run.clone(),
        });
        wake.0.notify_one();
    }
    Ok((Status::Accepted, Json(queued)))
}

/// `POST /packets/{id}/approval`: queues the `approve_packet` run that asks the user to
/// approve a `ready` packet, or returns the one already waiting.
#[utoipa::path(
    post,
    path = "/packets/{id}/approval",
    tag = "sourcing",
    operation_id = "request_packet_approval",
    params(("id" = i64, Path, description = "The packet's id")),
    request_body = RequestApprovalRequest,
    responses(
        (status = 202, description = "The `approve_packet` run: the one already waiting (`queued` or `paused`), `running` or `awaiting_approval` with `reused: true`, or a new one (`paused` while the kill switch is on). It parks with one `pending` action for the packet's draft", body = PacketQueued),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No packet has this id"),
        (status = 409, description = "The packet is not `ready`; nothing was queued"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 500, description = "The run could not be queued"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/packets/<id>/approval", data = "<body>")]
pub async fn request_packet_approval(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<RequestApprovalRequest>, json::Error<'_>>,
) -> Result<(Status, Json<PacketQueued>), ApiError> {
    let RequestApprovalRequest {} = parsed(body)?;
    let what = "request a packet's approval";
    let mut tx = pool.begin().await.map_err(internal(what))?;
    let queued = packet_actions::request_approval(&mut tx, bus, id).await;
    packet_queued(tx, bus, wake, queued, what).await
}

/// `POST /packets/{id}/applied`: queues the `apply_packet` run that records that the user
/// applied with an `approved` packet, or returns the one already waiting or running.
#[utoipa::path(
    post,
    path = "/packets/{id}/applied",
    tag = "sourcing",
    operation_id = "mark_packet_applied",
    params(("id" = i64, Path, description = "The packet's id")),
    request_body = MarkAppliedRequest,
    responses(
        (status = 202, description = "The `apply_packet` run: the one already waiting (`queued` or `paused`) or `running` with `reused: true`, or a new one (`paused` while the kill switch is on). It creates or updates the posting's application; nothing is submitted", body = PacketQueued),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No packet has this id"),
        (status = 409, description = "The packet is not `approved`, or its approved resume was deleted; nothing was queued"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 500, description = "The run could not be queued"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/packets/<id>/applied", data = "<body>")]
pub async fn mark_packet_applied(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<MarkAppliedRequest>, json::Error<'_>>,
) -> Result<(Status, Json<PacketQueued>), ApiError> {
    let MarkAppliedRequest {} = parsed(body)?;
    let what = "mark a packet applied";
    let mut tx = pool.begin().await.map_err(internal(what))?;
    let queued = packet_actions::queue_apply(&mut tx, bus, id).await;
    packet_queued(tx, bus, wake, queued, what).await
}
