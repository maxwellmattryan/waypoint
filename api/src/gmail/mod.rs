//! Gmail sync (see docs/gmail-sync.md): a read-only Gmail client and a paced,
//! single-flight sync on the job queue that stores messages with [`crate::email::ingest`].
//!
//! Routes: `GET /gmail/sync` reports the sync's status and `POST /gmail/sync` ("Sync now")
//! starts a run or returns the pending one.

pub mod client;
pub mod sync;

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::events::EventBus;
use crate::google::Google;
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::{GMAIL_SYNC, Job, Wake};

/// What the 409 from `POST /gmail/sync` says.
pub const NOT_CONNECTED_MESSAGE: &str = "Connect Google before syncing Gmail.";

/// Whether a sync is pending, from the `gmail_sync` jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncActivity {
    /// No `gmail_sync` job is queued or running.
    Idle,
    /// A page waits to start (maybe until `next_run_at`, after a rate limit).
    Queued,
    /// A page is running.
    Running,
}

/// The `GET /gmail/sync` body.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GmailSyncStatus {
    pub activity: SyncActivity,
    /// When the queued page may start; `null` unless `activity` is `queued`.
    #[schema(required = true)]
    pub next_run_at: Option<DateTime<Utc>>,
    /// The current or most recent run's mode; `null` before the first run.
    #[schema(required = true)]
    pub mode: Option<SyncMode>,
    #[schema(required = true)]
    pub run_started_at: Option<DateTime<Utc>>,
    /// When a run last finished its last page.
    #[schema(required = true)]
    pub last_finished_at: Option<DateTime<Utc>>,
    /// Why the last page didn't succeed; cleared when a run finishes.
    #[schema(required = true)]
    pub last_error: Option<String>,
    #[schema(required = true)]
    pub last_error_at: Option<DateTime<Utc>>,
    /// The current or most recent run's counts; all 0 before the first run.
    pub run: SyncCounts,
    /// Stored emails from Gmail.
    pub emails_stored: i64,
}

/// The `POST /gmail/sync` body.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GmailSyncQueued {
    /// The queued or running page job.
    pub job: Job,
    /// `true` when a sync was already queued or running and nothing new was enqueued.
    pub reused: bool,
}

type StateRow = (
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<DateTime<Utc>>,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
);

fn internal(what: &'static str) -> impl FnOnce(sqlx::Error) -> Status {
    move |err| {
        rocket::warn!("gmail: cannot {what}: {err}");
        Status::InternalServerError
    }
}

/// Reports the Gmail sync's status. Never calls Google or decrypts a token.
#[utoipa::path(
    get,
    path = "/gmail/sync",
    tag = "gmail",
    operation_id = "get_gmail_sync",
    responses(
        (status = 200, description = "The Gmail sync's status", body = GmailSyncStatus),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The status could not be read"),
    )
)]
#[rocket::get("/gmail/sync")]
pub async fn status(
    _host: LocalHost,
    pool: &State<PgPool>,
) -> Result<Json<GmailSyncStatus>, Status> {
    let pending: Option<(String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT status, run_at FROM jobs WHERE kind = $1 AND status IN ('queued', 'running') \
         ORDER BY status = 'running' DESC, id LIMIT 1",
    )
    .bind(GMAIL_SYNC)
    .fetch_optional(pool.inner())
    .await
    .map_err(internal("read the sync jobs"))?;
    let row: Option<StateRow> = sqlx::query_as(
        "SELECT run_mode, run_started_at, last_finished_at, last_error, last_error_at, \
         run_seen, run_inserted, run_updated, run_oversized, run_failed, run_vanished \
         FROM gmail_sync_state WHERE id",
    )
    .fetch_optional(pool.inner())
    .await
    .map_err(internal("read the sync state"))?;
    let emails_stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM emails WHERE source = 'gmail'")
            .fetch_one(pool.inner())
            .await
            .map_err(internal("count the Gmail emails"))?;

    let (activity, next_run_at) = match pending {
        None => (SyncActivity::Idle, None),
        Some((status, _)) if status == "running" => (SyncActivity::Running, None),
        Some((_, run_at)) => (SyncActivity::Queued, Some(run_at)),
    };
    let mut status = GmailSyncStatus {
        activity,
        next_run_at,
        mode: None,
        run_started_at: None,
        last_finished_at: None,
        last_error: None,
        last_error_at: None,
        run: SyncCounts::default(),
        emails_stored,
    };
    if let Some((mode, started, finished, error, error_at, seen, ins, upd, over, fail, gone)) = row
    {
        status.mode = mode.as_deref().and_then(SyncMode::parse);
        status.run_started_at = started;
        status.last_finished_at = finished;
        status.last_error = error;
        status.last_error_at = error_at;
        status.run = SyncCounts {
            seen,
            inserted: ins,
            updated: upd,
            oversized: over,
            failed: fail,
            vanished: gone,
        };
    }
    Ok(Json(status))
}

/// "Sync now": starts a Gmail sync, or returns the one already queued or running (a
/// queued page waiting out a delay is pulled forward to now).
#[utoipa::path(
    post,
    path = "/gmail/sync",
    tag = "gmail",
    operation_id = "sync_gmail",
    responses(
        (status = 202, description = "The first page's job: a new one, or (`reused: true`) the sync already queued or running", body = GmailSyncQueued),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 409, description = "Google isn't configured, connected, or needs reconnecting", body = String, content_type = "text/plain"),
        (status = 500, description = "The sync could not be queued"),
    )
)]
#[rocket::post("/gmail/sync")]
pub async fn sync_now(
    _host: LocalHost,
    _site: NotCrossSite,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    google: &State<Google>,
) -> Result<(Status, Json<GmailSyncQueued>), (Status, &'static str)> {
    match sync::request(pool, bus, wake, google, sync::Trigger::Manual).await {
        Ok(sync::Requested { job, reused }) => {
            Ok((Status::Accepted, Json(GmailSyncQueued { job, reused })))
        }
        Err(sync::RequestError::NotConnected) => Err((Status::Conflict, NOT_CONNECTED_MESSAGE)),
        Err(sync::RequestError::Database(err)) => {
            rocket::warn!("gmail: cannot queue a sync: {err}");
            Err((
                Status::InternalServerError,
                "The Gmail sync could not be started.",
            ))
        }
    }
}

/// How a sync run reads the mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// No stored `history_id`, or another account: lists the backfill window.
    Backfill,
    /// The stored history expired: lists the backfill window again, keeping stored rows.
    Resync,
    /// Reads `history.list` from the stored `history_id`.
    Incremental,
}

impl SyncMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backfill => "backfill",
            Self::Resync => "resync",
            Self::Incremental => "incremental",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "backfill" => Some(Self::Backfill),
            "resync" => Some(Self::Resync),
            "incremental" => Some(Self::Incremental),
            _ => None,
        }
    }
}

/// What a page or a run did with the messages it listed. `seen` is the sum of the rest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SyncCounts {
    pub seen: i32,
    /// Stored as new rows.
    pub inserted: i32,
    /// Already stored; refreshed in place.
    pub updated: i32,
    /// Over the size cap; not stored.
    pub oversized: i32,
    /// Didn't parse or came back with another id; not stored.
    pub failed: i32,
    /// Deleted between listing and fetching.
    pub vanished: i32,
}

/// Gmail sync settings. Code defaults; tests override them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The Gmail API origin.
    pub base_url: String,
    /// How far back a backfill or resync reads (`newer_than:<days>d`).
    pub backfill_days: u32,
    /// Most messages a backfill or resync lists.
    pub max_messages: u32,
    /// Ids per page (`maxResults`).
    pub page_size: u32,
    /// Delay before the next page's job may start.
    pub page_delay: Duration,
    /// Largest decoded message stored; bigger ones count as `oversized`.
    pub max_message_bytes: usize,
    /// Startup catch-up runs when the last sync finished longer ago than this.
    pub stale_after: Duration,
}

pub const BASE_URL: &str = "https://gmail.googleapis.com";

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.into(),
            backfill_days: 30,
            max_messages: 1000,
            page_size: 100,
            page_delay: Duration::from_secs(2),
            max_message_bytes: 5 * 1024 * 1024,
            stale_after: Duration::from_secs(10 * 60),
        }
    }
}
