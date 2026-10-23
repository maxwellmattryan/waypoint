//! The Gmail sync engine on the job queue. A run is a chain of `gmail_sync` jobs, one page
//! each: [`request`] starts one (single-flight), [`run_page`] is the handler, and each page
//! enqueues its successor inside the transaction that records its counts. The cursor lives
//! in the job payload; counts, the stored `history_id` and the last error live in
//! `gmail_sync_state`.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::client::{GmailClient, GmailError};
use super::{Config, SyncCounts, SyncMode};
use crate::claude::{MAX_ERROR_BYTES, head};
use crate::email::{self, IngestError, NewEmail, Source};
use crate::events::EventBus;
use crate::google::{self, Google, GoogleError};
use crate::jobs::{self, COLUMNS, GMAIL_SYNC, GMAIL_SYNC_MAX_ATTEMPTS, Job, Wake};

/// Serializes [`request`] calls.
const LOCK: &str = "SELECT pg_advisory_xact_lock(hashtext('waypoint.gmail_sync'))";
/// History entries with any of these labels are never fetched.
const SKIPPED_LABELS: [&str; 3] = ["DRAFT", "SPAM", "TRASH"];

/// Why a run was asked for: the start job's payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// `POST /gmail/sync`.
    Manual,
    /// The worker's catch-up after downtime.
    Startup,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Startup => "startup",
        }
    }
}

/// Why [`request`] started nothing.
#[derive(Debug)]
pub enum RequestError {
    /// Google isn't configured, or no account is connected (or it needs reconnecting).
    NotConnected,
    Database(sqlx::Error),
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => f.write_str("no connected Google account to sync"),
            Self::Database(err) => write!(f, "database error: {err}"),
        }
    }
}

impl std::error::Error for RequestError {}

impl From<sqlx::Error> for RequestError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}

/// The run [`request`] returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Requested {
    /// The queued or running page job.
    pub job: Job,
    /// `true` when a run was already queued or running and nothing new was enqueued.
    pub reused: bool,
}

/// Starts a sync run, or returns the one already queued or running (pulling a delayed
/// queued page forward to now). The only way to start a run: it holds an advisory lock,
/// so two callers can't both enqueue.
pub async fn request(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    google: &Google,
    trigger: Trigger,
) -> Result<Requested, RequestError> {
    if !google.config.is_configured() {
        return Err(RequestError::NotConnected);
    }
    let mut tx = pool.begin().await?;
    sqlx::query(LOCK).execute(&mut *tx).await?;
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM google_accounts WHERE id")
        .fetch_optional(&mut *tx)
        .await?;
    if status.as_deref() != Some("connected") {
        return Err(RequestError::NotConnected);
    }
    let pending: Option<Job> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM jobs WHERE kind = $1 AND status IN ('queued', 'running') \
         ORDER BY status = 'running' DESC, id LIMIT 1"
    ))
    .bind(GMAIL_SYNC)
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
                GMAIL_SYNC,
                json!({"trigger": trigger.as_str()}),
                GMAIL_SYNC_MAX_ATTEMPTS,
            )
            .await?;
            Requested { job, reused: false }
        }
    };
    tx.commit().await?;
    wake.0.notify_one();
    Ok(requested)
}

/// Startup catch-up: when Google is connected and the last sync finished longer than
/// `stale_after` ago (or never), requests a run. An orphaned page the worker just requeued
/// already is one, so it's reused. `None` when nothing was requested.
pub async fn catch_up(
    pool: &PgPool,
    bus: &EventBus,
    google: &Google,
) -> Result<Option<Requested>, sqlx::Error> {
    if !google.config.is_configured() {
        return Ok(None);
    }
    let stale: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM gmail_sync_state WHERE id \
         AND last_finished_at > now() - $1 * interval '1 millisecond')",
    )
    .bind(millis(google.config.gmail.stale_after))
    .fetch_one(pool)
    .await?;
    if !stale {
        return Ok(None);
    }
    // The worker claims the job next, so the wake goes nowhere.
    match request(pool, bus, &Wake::default(), google, Trigger::Startup).await {
        Ok(requested) => Ok(Some(requested)),
        Err(RequestError::NotConnected) => Ok(None),
        Err(RequestError::Database(err)) => Err(err),
    }
}

/// What one page job decided; `jobs::worker` maps it onto its outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum PageOutcome {
    Done(Value),
    Retry(String),
    /// Retry no sooner than this (Gmail's `Retry-After`).
    RetryAfter(String, Duration),
    Fail(String),
}

/// A page that didn't finish.
enum Failure {
    /// Google isn't usable any more: the run ends and the job succeeds.
    Stop(String),
    Retry(String),
    RetryAfter(String, Duration),
    Fail(String),
}

impl Failure {
    fn message(&self) -> &str {
        match self {
            Self::Stop(m) | Self::Retry(m) | Self::RetryAfter(m, _) | Self::Fail(m) => m,
        }
    }
}

/// Where a run is: the payload of every page job after the start job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cursor {
    mode: SyncMode,
    /// Backfill and resync: the profile's `historyId` from the run's start, stored when the
    /// run finishes. Incremental: the stored id `history.list` reads from.
    start_history_id: String,
    page_token: Option<String>,
    /// 1-based.
    page: u32,
    /// Messages listed so far (backfill and resync).
    listed: u32,
}

/// The `gmail_sync` handler: runs one page (a start job first picks the mode). Every
/// outcome but success is recorded as the state's `last_error`.
pub async fn run_page(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    google: &Google,
    job: &Job,
) -> PageOutcome {
    let failure = match page(pool, bus, wake, google, &job.payload).await {
        Ok(result) => return PageOutcome::Done(result),
        Err(failure) => failure,
    };
    rocket::warn!("gmail: sync job {} failed: {}", job.id, failure.message());
    if let Err(err) = record_error(pool, failure.message()).await {
        rocket::warn!("gmail: cannot record the sync error: {err}");
    }
    match failure {
        Failure::Stop(reason) => PageOutcome::Done(json!({"stopped": reason})),
        Failure::Retry(error) => PageOutcome::Retry(error),
        Failure::RetryAfter(error, delay) => PageOutcome::RetryAfter(error, delay),
        Failure::Fail(error) => PageOutcome::Fail(error),
    }
}

async fn page(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    google: &Google,
    payload: &Value,
) -> Result<Value, Failure> {
    let cursor = match payload.get("mode") {
        Some(_) => Some(
            Cursor::deserialize(payload)
                .map_err(|err| Failure::Fail(format!("invalid {GMAIL_SYNC} payload: {err}")))?,
        ),
        None => None,
    };
    let token = google::access_token(pool, google)
        .await
        .map_err(token_failure)?;
    let config = &google.config.gmail;
    let client = GmailClient::new(google.http(), &config.base_url, &token);
    let cursor = match cursor {
        Some(cursor) => cursor,
        None => start(pool, &client).await?,
    };
    process(pool, bus, wake, &client, config, cursor).await
}

/// The start job: reads the profile, picks the mode and resets the run in the state row.
async fn start(pool: &PgPool, client: &GmailClient<'_>) -> Result<Cursor, Failure> {
    let profile = match client.profile().await {
        Ok(profile) => profile,
        Err(err) => return Err(gmail_failure(pool, err).await),
    };
    let stored: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT email_address, history_id FROM gmail_sync_state WHERE id")
            .fetch_optional(pool)
            .await
            .map_err(db_failure)?;
    let (mode, start_history_id) = match stored {
        Some((Some(address), Some(history_id))) if address == profile.email_address => {
            (SyncMode::Incremental, history_id)
        }
        _ => (SyncMode::Backfill, profile.history_id),
    };
    // Another account's history_id is dropped, so a run that fails part-way can't resume
    // incrementally from it.
    sqlx::query(
        "INSERT INTO gmail_sync_state (id, email_address, run_mode, run_started_at) \
         VALUES (true, $1, $2, now()) \
         ON CONFLICT (id) DO UPDATE SET \
           history_id = CASE WHEN gmail_sync_state.email_address IS DISTINCT FROM \
             EXCLUDED.email_address THEN NULL ELSE gmail_sync_state.history_id END, \
           email_address = EXCLUDED.email_address, run_mode = EXCLUDED.run_mode, \
           run_started_at = now(), run_seen = 0, run_inserted = 0, run_updated = 0, \
           run_oversized = 0, run_failed = 0, run_vanished = 0, updated_at = now()",
    )
    .bind(&profile.email_address)
    .bind(mode.as_str())
    .execute(pool)
    .await
    .map_err(db_failure)?;
    Ok(Cursor {
        mode,
        start_history_id,
        page_token: None,
        page: 1,
        listed: 0,
    })
}

/// One listed page: its ids, the next page's token and, for incremental, its `historyId`.
struct Listed {
    ids: Vec<String>,
    next_page_token: Option<String>,
    history_id: Option<String>,
}

/// Lists, fetches and ingests one page, then records it and enqueues the next page or
/// finishes the run.
async fn process(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    client: &GmailClient<'_>,
    config: &Config,
    mut cursor: Cursor,
) -> Result<Value, Failure> {
    let listed = match cursor.mode {
        SyncMode::Backfill | SyncMode::Resync => list_window(pool, client, config, &cursor).await?,
        SyncMode::Incremental => {
            match client
                .history(
                    &cursor.start_history_id,
                    cursor.page_token.as_deref(),
                    config.page_size,
                )
                .await
            {
                Ok(page) => Listed {
                    ids: page
                        .history
                        .into_iter()
                        .flat_map(|record| record.messages_added)
                        .map(|added| added.message)
                        .filter(|m| {
                            !m.label_ids
                                .iter()
                                .any(|l| SKIPPED_LABELS.contains(&l.as_str()))
                        })
                        .map(|m| m.id)
                        .collect(),
                    next_page_token: page.next_page_token,
                    history_id: Some(page.history_id),
                },
                Err(GmailError::HistoryExpired) => {
                    rocket::info!("gmail: the stored history expired; resyncing");
                    let profile = match client.profile().await {
                        Ok(profile) => profile,
                        Err(err) => return Err(gmail_failure(pool, err).await),
                    };
                    sqlx::query(
                        "UPDATE gmail_sync_state SET run_mode = 'resync', updated_at = now() \
                         WHERE id",
                    )
                    .execute(pool)
                    .await
                    .map_err(db_failure)?;
                    cursor = Cursor {
                        mode: SyncMode::Resync,
                        start_history_id: profile.history_id,
                        page_token: None,
                        page: cursor.page,
                        listed: 0,
                    };
                    list_window(pool, client, config, &cursor).await?
                }
                Err(err) => return Err(gmail_failure(pool, err).await),
            }
        }
    };

    let mut unique = HashSet::new();
    let ids: Vec<String> = listed
        .ids
        .into_iter()
        .filter(|id| unique.insert(id.clone()))
        .collect();
    let listed_so_far = match cursor.mode {
        SyncMode::Incremental => cursor.listed,
        _ => cursor
            .listed
            .saturating_add(u32::try_from(ids.len()).unwrap_or(u32::MAX)),
    };
    let more = listed.next_page_token.is_some()
        && (cursor.mode == SyncMode::Incremental || listed_so_far < config.max_messages);

    let counts = fetch_and_ingest(pool, bus, wake, client, config, &ids).await?;

    let mut tx = pool.begin().await.map_err(db_failure)?;
    let exists: Option<bool> =
        sqlx::query_scalar("SELECT true FROM gmail_sync_state WHERE id FOR UPDATE")
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_failure)?;
    if exists.is_none() {
        rocket::info!("gmail: the sync state was deleted; the run stops");
        return Ok(json!({"stopped": "the sync state was deleted"}));
    }
    sqlx::query(
        "UPDATE gmail_sync_state SET run_seen = run_seen + $1, \
           run_inserted = run_inserted + $2, run_updated = run_updated + $3, \
           run_oversized = run_oversized + $4, run_failed = run_failed + $5, \
           run_vanished = run_vanished + $6, updated_at = now() WHERE id",
    )
    .bind(counts.seen)
    .bind(counts.inserted)
    .bind(counts.updated)
    .bind(counts.oversized)
    .bind(counts.failed)
    .bind(counts.vanished)
    .execute(&mut *tx)
    .await
    .map_err(db_failure)?;
    let next_job_id = if more {
        let next = Cursor {
            mode: cursor.mode,
            start_history_id: cursor.start_history_id.clone(),
            page_token: listed.next_page_token,
            page: cursor.page.saturating_add(1),
            listed: listed_so_far,
        };
        let payload = serde_json::to_value(&next)
            .map_err(|err| Failure::Fail(format!("cannot serialize the next page: {err}")))?;
        let job = jobs::enqueue_tx_after(
            &mut tx,
            bus,
            GMAIL_SYNC,
            payload,
            GMAIL_SYNC_MAX_ATTEMPTS,
            config.page_delay,
        )
        .await
        .map_err(db_failure)?;
        Some(job.id)
    } else {
        let history_id = match cursor.mode {
            SyncMode::Incremental => listed.history_id.unwrap_or(cursor.start_history_id),
            SyncMode::Backfill | SyncMode::Resync => cursor.start_history_id,
        };
        sqlx::query(
            "UPDATE gmail_sync_state SET history_id = $1, last_finished_at = now(), \
             last_error = NULL, last_error_at = NULL, updated_at = now() WHERE id",
        )
        .bind(&history_id)
        .execute(&mut *tx)
        .await
        .map_err(db_failure)?;
        None
    };
    tx.commit().await.map_err(db_failure)?;

    Ok(json!({
        "mode": cursor.mode.as_str(),
        "page": cursor.page,
        "seen": counts.seen,
        "inserted": counts.inserted,
        "updated": counts.updated,
        "oversized": counts.oversized,
        "failed": counts.failed,
        "vanished": counts.vanished,
        "next_job_id": next_job_id,
    }))
}

/// A backfill or resync page: `newer_than:<days>d -in:drafts`, newest first, up to the
/// run's remaining `max_messages`.
async fn list_window(
    pool: &PgPool,
    client: &GmailClient<'_>,
    config: &Config,
    cursor: &Cursor,
) -> Result<Listed, Failure> {
    let remaining = config.max_messages.saturating_sub(cursor.listed);
    if remaining == 0 {
        return Ok(Listed {
            ids: Vec::new(),
            next_page_token: None,
            history_id: None,
        });
    }
    let q = format!("newer_than:{}d -in:drafts", config.backfill_days);
    let max = config.page_size.clamp(1, remaining);
    match client
        .list_messages(&q, cursor.page_token.as_deref(), max)
        .await
    {
        Ok(list) => Ok(Listed {
            ids: list.messages.into_iter().map(|m| m.id).collect(),
            next_page_token: list.next_page_token,
            history_id: None,
        }),
        Err(err) => Err(gmail_failure(pool, err).await),
    }
}

/// Fetches each id one at a time and ingests it. Only these ids are written.
async fn fetch_and_ingest(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    client: &GmailClient<'_>,
    config: &Config,
    ids: &[String],
) -> Result<SyncCounts, Failure> {
    let mut counts = SyncCounts::default();
    if ids.is_empty() {
        return Ok(counts);
    }
    let known = email::known_companies(pool).await.map_err(db_failure)?;
    for id in ids {
        match client.get_raw(id, config.max_message_bytes).await {
            Ok(message) if message.id != *id => {
                rocket::warn!("gmail: message {id} came back with another id; skipped");
                counts.failed += 1;
            }
            Ok(message) => {
                let new = NewEmail {
                    source: Source::Gmail,
                    provider_message_id: id.clone(),
                    provider_thread_id: message.thread_id,
                    labels: message.label_ids,
                    raw: message.raw,
                };
                match email::ingest(pool, bus, wake, new, &known).await {
                    Ok(stored) if stored.inserted => counts.inserted += 1,
                    Ok(_) => counts.updated += 1,
                    Err(IngestError::Parse(_)) => {
                        rocket::warn!("gmail: message {id} does not parse; skipped");
                        counts.failed += 1;
                    }
                    Err(IngestError::Db(err)) => return Err(db_failure(err)),
                }
            }
            Err(GmailError::Oversized) => {
                rocket::info!("gmail: message {id} is over the size cap; skipped");
                counts.oversized += 1;
            }
            Err(GmailError::NotFound) => counts.vanished += 1,
            Err(err) => return Err(gmail_failure(pool, err).await),
        }
    }
    counts.seen =
        counts.inserted + counts.updated + counts.oversized + counts.failed + counts.vanished;
    Ok(counts)
}

/// A missing, unconfigured or revoked account ends the run; anything else retries.
fn token_failure(err: GoogleError) -> Failure {
    match err {
        GoogleError::NotConfigured(_)
        | GoogleError::NotConnected
        | GoogleError::ReconnectRequired => Failure::Stop(err.to_string()),
        GoogleError::Http(_) | GoogleError::Database(_) => Failure::Retry(err.to_string()),
    }
}

/// A 401 expires the stored token so the retry refreshes it; a rate limit retries after
/// `Retry-After`; everything else retries with the normal backoff.
async fn gmail_failure(pool: &PgPool, err: GmailError) -> Failure {
    match err {
        GmailError::Unauthorized => {
            if let Err(db) = google::expire_access_token(pool).await {
                rocket::warn!("gmail: cannot expire the access token: {db}");
            }
            Failure::Retry(err.to_string())
        }
        GmailError::RateLimited { retry_after } => {
            Failure::RetryAfter(err.to_string(), retry_after.unwrap_or_default())
        }
        other => Failure::Retry(other.to_string()),
    }
}

fn db_failure(err: sqlx::Error) -> Failure {
    Failure::Retry(format!("database error: {err}"))
}

/// Sets `last_error`. An `UPDATE`, so a wiped state stays wiped.
async fn record_error(pool: &PgPool, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE gmail_sync_state SET last_error = $1, last_error_at = now(), \
         updated_at = now() WHERE id",
    )
    .bind(head(error, MAX_ERROR_BYTES))
    .execute(pool)
    .await
    .map(|_| ())
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
