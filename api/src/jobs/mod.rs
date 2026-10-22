//! The Postgres-backed job queue: one `jobs` row per unit of background work, run by
//! the in-process [`Worker`]. Each query here is one SQL statement that returns the
//! updated row.

mod worker;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::serde::json::Json;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::Notify;

pub use self::worker::{Worker, backoff};
use crate::claude::{MAX_ERROR_BYTES, head};
use crate::events::{Event, EventBus};
use crate::host::LocalHost;

/// The `test_prompt` job kind: payload `{"prompt": string}`, one `claude -p` call.
pub const TEST_PROMPT: &str = "test_prompt";
/// The `capture_extract` job kind: payload `{"url": string, "text": string}`, one
/// structured `claude -p` call whose `result` is a `Capture` (see `capture::llm`).
pub const CAPTURE_EXTRACT: &str = "capture_extract";

/// One `jobs` row. This is also the `GET /jobs/<id>` body; the payload is not echoed.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    #[serde(skip)]
    #[schema(ignore)]
    pub payload: Value,
    /// `queued`, `running`, `succeeded` or `failed`.
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    /// The earliest time the job may start.
    pub run_at: DateTime<Utc>,
    // The `Option` fields are always serialized (as `null` when empty), so they are
    // required in the schema.
    #[schema(required = true)]
    pub last_error: Option<String>,
    /// Any JSON value the job produced.
    #[schema(required = true)]
    pub result: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[schema(required = true)]
    pub finished_at: Option<DateTime<Utc>>,
}

const COLUMNS: &str = "id, kind, payload, status, attempts, max_attempts, run_at, last_error, \
                       result, created_at, updated_at, finished_at";

/// Queue settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Most jobs running at once.
    pub concurrency: usize,
    /// How long the worker sleeps when no job is ready and nothing was enqueued.
    pub poll_interval: Duration,
    /// First retry delay; it doubles per attempt up to `retry_cap`.
    pub retry_base: Duration,
    pub retry_cap: Duration,
    /// How long the queue pauses after a usage limit.
    pub usage_limit_pause: Duration,
    /// Attempts a new job gets.
    pub max_attempts: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            concurrency: 2,
            poll_interval: Duration::from_secs(2),
            retry_base: Duration::from_secs(10),
            retry_cap: Duration::from_secs(10 * 60),
            usage_limit_pause: Duration::from_secs(15 * 60),
            max_attempts: 3,
        }
    }
}

impl Config {
    /// Reads `WAYPOINT_JOB_CONCURRENCY` (default 2) and
    /// `WAYPOINT_JOB_USAGE_LIMIT_PAUSE_SECS` (default 900) over the defaults.
    pub fn from_env() -> Result<Self, String> {
        let env = |key: &str| std::env::var(key).ok();
        Self::from_values(
            env("WAYPOINT_JOB_CONCURRENCY").as_deref(),
            env("WAYPOINT_JOB_USAGE_LIMIT_PAUSE_SECS").as_deref(),
        )
    }

    fn from_values(concurrency: Option<&str>, pause_secs: Option<&str>) -> Result<Self, String> {
        let mut config = Self::default();
        if let Some(n) = parse_positive("WAYPOINT_JOB_CONCURRENCY", "integer", concurrency)? {
            config.concurrency = usize::try_from(n)
                .map_err(|_| format!("WAYPOINT_JOB_CONCURRENCY is too large, got {n}"))?;
        }
        if let Some(secs) = parse_positive(
            "WAYPOINT_JOB_USAGE_LIMIT_PAUSE_SECS",
            "number of seconds",
            pause_secs,
        )? {
            config.usage_limit_pause = Duration::from_secs(secs);
        }
        Ok(config)
    }
}

/// Parses an optional positive integer setting; blank means unset.
fn parse_positive(key: &str, what: &str, value: Option<&str>) -> Result<Option<u64>, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(raw) => match raw.parse::<u64>() {
            Ok(n) if n > 0 => Ok(Some(n)),
            _ => Err(format!("{key} must be a positive {what}, got {raw:?}")),
        },
    }
}

/// Wakes the worker when a job is enqueued, so it doesn't wait for the next poll.
#[derive(Debug, Clone, Default)]
pub struct Wake(pub Arc<Notify>);

/// Enqueues a job with the default number of attempts. See [`enqueue_with`].
pub async fn enqueue(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    kind: &str,
    payload: Value,
) -> Result<Job, sqlx::Error> {
    enqueue_with(
        pool,
        bus,
        wake,
        kind,
        payload,
        Config::default().max_attempts,
    )
    .await
}

/// Inserts a `queued` job that may run now, publishes `job.queued` and wakes the worker.
pub async fn enqueue_with(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    kind: &str,
    payload: Value,
    max_attempts: i32,
) -> Result<Job, sqlx::Error> {
    // `job.queued` goes out before the commit, so the worker can't claim the row (and
    // publish `job.started`) first.
    let mut tx = pool.begin().await?;
    let job: Job = sqlx::query_as(&format!(
        "INSERT INTO jobs (kind, payload, max_attempts) VALUES ($1, $2, $3) RETURNING {COLUMNS}"
    ))
    .bind(kind)
    .bind(payload)
    .bind(max_attempts)
    .fetch_one(&mut *tx)
    .await?;
    bus.publish(Event::JobQueued { job: job.clone() });
    tx.commit().await?;
    wake.0.notify_one();
    Ok(job)
}

pub async fn get(pool: &PgPool, id: i64) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM jobs WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Marks the oldest ready `queued` job `running` and counts the attempt. Rows another
/// transaction has locked are skipped, never waited on.
pub async fn claim(pool: &PgPool) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as(&format!(
        "UPDATE jobs SET status = 'running', attempts = attempts + 1, updated_at = now() \
         WHERE id = ( \
             SELECT id FROM jobs WHERE status = 'queued' AND run_at <= now() \
             ORDER BY run_at, id LIMIT 1 FOR UPDATE SKIP LOCKED \
         ) RETURNING {COLUMNS}"
    ))
    .fetch_optional(pool)
    .await
}

/// Puts every `running` job back in the queue. Called once when the worker starts: the
/// API is a single process, so any running row then was orphaned by a crash or a kill.
pub async fn requeue_orphans(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let done = sqlx::query(
        "UPDATE jobs SET status = 'queued', updated_at = now() WHERE status = 'running'",
    )
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

pub async fn succeed(pool: &PgPool, id: i64, result: &Value) -> Result<Job, sqlx::Error> {
    sqlx::query_as(&format!(
        "UPDATE jobs SET status = 'succeeded', result = $2, finished_at = now(), \
         updated_at = now() WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(result)
    .fetch_one(pool)
    .await
}

/// Requeues a failed attempt to run again after `delay`.
pub async fn retry(
    pool: &PgPool,
    id: i64,
    error: &str,
    delay: Duration,
) -> Result<Job, sqlx::Error> {
    sqlx::query_as(&format!(
        "UPDATE jobs SET status = 'queued', run_at = now() + $3 * interval '1 millisecond', \
         last_error = $2, updated_at = now() WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(head(error, MAX_ERROR_BYTES))
    .bind(millis(delay))
    .fetch_one(pool)
    .await
}

/// Requeues a job that hit the usage limit to run after `pause`, giving its attempt back.
pub async fn defer(
    pool: &PgPool,
    id: i64,
    error: &str,
    pause: Duration,
) -> Result<Job, sqlx::Error> {
    sqlx::query_as(&format!(
        "UPDATE jobs SET status = 'queued', attempts = GREATEST(attempts - 1, 0), \
         run_at = now() + $3 * interval '1 millisecond', last_error = $2, updated_at = now() \
         WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(head(error, MAX_ERROR_BYTES))
    .bind(millis(pause))
    .fetch_one(pool)
    .await
}

pub async fn fail(pool: &PgPool, id: i64, error: &str) -> Result<Job, sqlx::Error> {
    sqlx::query_as(&format!(
        "UPDATE jobs SET status = 'failed', last_error = $2, finished_at = now(), \
         updated_at = now() WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(head(error, MAX_ERROR_BYTES))
    .fetch_one(pool)
    .await
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// `GET /jobs/<id>`: one job's current state.
#[utoipa::path(
    get,
    path = "/jobs/{id}",
    tag = "jobs",
    params(("id" = i64, Path, description = "The job's id")),
    responses(
        (status = 200, description = "The job's current state", body = Job),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No job has this id"),
        (status = 500, description = "The job could not be read"),
    )
)]
#[rocket::get("/jobs/<id>")]
pub async fn show(_host: LocalHost, id: i64, pool: &State<PgPool>) -> Result<Json<Job>, Status> {
    match get(pool, id).await {
        Ok(Some(job)) => Ok(Json(job)),
        Ok(None) => Err(Status::NotFound),
        Err(err) => {
            rocket::warn!("jobs: cannot read job {id}: {err}");
            Err(Status::InternalServerError)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_env_values_and_rejects_garbage() {
        assert_eq!(Config::from_values(None, Some(" ")), Ok(Config::default()));
        let config = Config::from_values(Some("4"), Some("60")).unwrap();
        assert_eq!(config.concurrency, 4);
        assert_eq!(config.usage_limit_pause, Duration::from_secs(60));
        for bad in ["0", "-1", "two"] {
            let err = Config::from_values(Some(bad), None).unwrap_err();
            assert!(err.contains("WAYPOINT_JOB_CONCURRENCY"), "{err}");
            let err = Config::from_values(None, Some(bad)).unwrap_err();
            assert!(err.contains("WAYPOINT_JOB_USAGE_LIMIT_PAUSE_SECS"), "{err}");
        }
    }
}
