//! Board sourcing: target companies' public Greenhouse, Lever and Ashby boards, kept in
//! `job_postings` by one `source_board` job per (source, board slug). `POST
//! /sourcing/boards` queues the job through [`request`], which reuses one already queued
//! or running for the same board; [`run`] is the job's handler. See `docs/sourcing.md`.

pub mod boards;
pub mod location;
pub mod postings;
pub mod store;
pub mod text;

use std::time::Instant;

use rocket::State;
use rocket::http::Status;
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::applications::{
    ApiError, FieldError, JsonContent, REQUIRED, ValidationErrors, internal, parsed,
};
use crate::capture::Capturer;
use crate::events::EventBus;
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::{self, COLUMNS, Job, SOURCE_BOARD, Wake};

/// Most characters in a board slug.
pub const MAX_BOARD: usize = 100;
const BAD_SOURCE: &str = "Must be greenhouse, lever or ashby";
const BAD_BOARD: &str = "Must be 1–100 letters, digits, hyphens or underscores";

/// A public job board this module reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Greenhouse,
    Lever,
    Ashby,
}

crate::memories::text_enum!(Source, "source", [
    Greenhouse => "greenhouse",
    Lever => "lever",
    Ashby => "ashby",
]);

impl Source {
    /// The exact lowercase name, as `as_str` gives it.
    pub fn parse(s: &str) -> Option<Self> {
        Self::try_from(s.to_string()).ok()
    }
}

/// Whether `board` is a slug the board APIs can take: 1–[`MAX_BOARD`] characters of
/// `[A-Za-z0-9_-]`. Nothing else reaches a board URL.
pub fn is_board_slug(board: &str) -> bool {
    (1..=MAX_BOARD).contains(&board.len())
        && board
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The `POST /sourcing/boards` body.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct SourceBoardRequest {
    /// `greenhouse`, `lever` or `ashby`. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub source: Option<String>,
    /// The board's slug, as in its board URL (`boards.greenhouse.io/<board>`,
    /// `jobs.lever.co/<board>`, `jobs.ashbyhq.com/<board>`): 1–100 letters, digits,
    /// hyphens or underscores. Trimmed; case is kept. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub board: Option<String>,
}

/// The 202 body.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct SourceBoardQueued {
    /// The queued or running `source_board` job; its `result` holds the counts.
    pub job: Job,
    /// `true` when a fetch of this board was already queued or running and nothing new
    /// was enqueued.
    pub reused: bool,
}

/// The job [`request`] returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Requested {
    pub job: Job,
    /// `true` when it was already queued or running.
    pub reused: bool,
}

/// Checks both fields, reporting every failing one at once.
fn validate(input: &SourceBoardRequest) -> Result<(Source, String), ApiError> {
    let mut errors = Vec::new();
    let error = |field: &str, message: &str| FieldError {
        field: field.into(),
        message: message.into(),
    };
    let source = match input.source.as_deref() {
        None => {
            errors.push(error("source", REQUIRED));
            None
        }
        Some(raw) => {
            let source = Source::parse(raw);
            if source.is_none() {
                errors.push(error("source", BAD_SOURCE));
            }
            source
        }
    };
    let board = input.board.as_deref().map(str::trim).unwrap_or_default();
    if board.is_empty() {
        errors.push(error("board", REQUIRED));
    } else if !is_board_slug(board) {
        errors.push(error("board", BAD_BOARD));
    }
    match source {
        Some(source) if errors.is_empty() => Ok((source, board.to_string())),
        _ => Err(ApiError::Invalid(ValidationErrors { errors })),
    }
}

/// Queues a fetch of one board, or returns the one already queued or running for it
/// (pulling a delayed queued job forward to now). It holds the board's advisory lock, the
/// one the job's save also takes, so two callers can't both enqueue.
pub async fn request(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    source: Source,
    board: &str,
) -> Result<Requested, sqlx::Error> {
    let mut tx = pool.begin().await?;
    store::lock(&mut tx, source, board).await?;
    let pending: Option<Job> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM jobs WHERE kind = $1 AND status IN ('queued', 'running') \
         AND payload->>'source' = $2 AND payload->>'board' = $3 \
         ORDER BY status = 'running' DESC, id LIMIT 1"
    ))
    .bind(SOURCE_BOARD)
    .bind(source.as_str())
    .bind(board)
    .fetch_optional(&mut *tx)
    .await?;
    let requested = match pending {
        Some(job) if job.status == "running" => Requested { job, reused: true },
        Some(job) => {
            let job = sqlx::query_as(&format!(
                "UPDATE jobs SET run_at = LEAST(run_at, now()), updated_at = now() \
                 WHERE id = $1 RETURNING {COLUMNS}"
            ))
            .bind(job.id)
            .fetch_one(&mut *tx)
            .await?;
            Requested { job, reused: true }
        }
        None => {
            let job = jobs::enqueue_tx(
                &mut tx,
                bus,
                SOURCE_BOARD,
                json!({"source": source.as_str(), "board": board}),
                jobs::Config::default().max_attempts,
            )
            .await?;
            Requested { job, reused: false }
        }
    };
    tx.commit().await?;
    wake.0.notify_one();
    Ok(requested)
}

/// What one `source_board` attempt decided; `jobs::worker` maps it onto its outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done(Value),
    Retry(String),
    Fail(String),
}

#[derive(Deserialize)]
struct Payload {
    source: String,
    board: String,
}

/// The `source_board` handler: fetches the whole board, then saves it in one
/// transaction. A failed or partial fetch, or a database error, writes nothing and
/// retries. The `result` is the counts.
pub async fn run(pool: &PgPool, capturer: &Capturer, job: &Job) -> Outcome {
    let parsed = Payload::deserialize(&job.payload).ok().and_then(|payload| {
        let source = Source::parse(&payload.source)?;
        is_board_slug(&payload.board).then_some((source, payload.board))
    });
    let Some((source, board)) = parsed else {
        return Outcome::Fail(format!("invalid {SOURCE_BOARD} payload"));
    };
    let name = source.as_str();
    let started = Instant::now();

    let fetched = match boards::fetch(capturer, source, &board).await {
        Ok(fetched) => fetched,
        Err(reason) => {
            rocket::warn!("sourcing: {name}/{board}: the fetch failed: {reason}");
            return Outcome::Retry(format!("the fetch failed: {reason}"));
        }
    };
    let saved = match store::save_board(pool, source, &board, &fetched.postings).await {
        Ok(saved) => saved,
        Err(err) => {
            rocket::warn!("sourcing: {name}/{board}: cannot save the postings: {err}");
            return Outcome::Retry(format!("cannot save the postings: {err}"));
        }
    };
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let boards::Fetched {
        fetched,
        skipped,
        pages,
        ..
    } = fetched;
    let store::Saved {
        inserted,
        updated,
        removed,
    } = saved;
    rocket::info!(
        "sourcing: {name}/{board}: {fetched} fetched, {inserted} new, {updated} updated, \
         {removed} removed, {skipped} skipped in {pages} page(s), {duration_ms}ms"
    );
    Outcome::Done(json!({
        "source": name,
        "board": board,
        "pages": pages,
        "fetched": fetched,
        "skipped": skipped,
        "inserted": inserted,
        "updated": updated,
        "removed": removed,
        "duration_ms": duration_ms,
    }))
}

/// Queues a fetch of one public job board into `job_postings`.
#[utoipa::path(
    post,
    path = "/sourcing/boards",
    tag = "sourcing",
    operation_id = "source_board",
    request_body = SourceBoardRequest,
    responses(
        (status = 202, description = "The board's `source_board` job: new, or the one already queued or running", body = SourceBoardQueued),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "The source or the board slug is invalid", body = ValidationErrors),
        (status = 500, description = "The job could not be queued"),
    )
)]
#[rocket::post("/sourcing/boards", data = "<body>")]
pub async fn source_board(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<SourceBoardRequest>, json::Error<'_>>,
) -> Result<(Status, Json<SourceBoardQueued>), ApiError> {
    let input = parsed(body)?;
    let (source, board) = validate(&input)?;
    let Requested { job, reused } = request(pool, bus, wake, source, &board)
        .await
        .map_err(internal("queue a board fetch"))?;
    Ok((Status::Accepted, Json(SourceBoardQueued { job, reused })))
}
