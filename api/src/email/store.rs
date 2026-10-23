//! Storing emails: [`ingest`] (parse, normalize, score, thread and upsert one message, and
//! queue its classification), [`list`] and [`known_companies`].

use std::fmt;

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{PgPool, Row};

use super::heuristics::{self, HEURISTICS_VERSION, HeuristicInput, KnownCompanies};
use super::mime::{self, ParseError};
use super::normalize::{self, NORMALIZER_VERSION};
use super::thread;
use super::{Ats, Source, VerdictKind};
use crate::events::{Event, EventBus};
use crate::jobs::Wake;
use crate::runs::Run;
use crate::workflows::classify_email::{self, KeyInput};

/// One raw message to store, with where it came from.
#[derive(Debug, Clone)]
pub struct NewEmail {
    pub source: Source,
    pub provider_message_id: String,
    /// The Gmail thread id, when known.
    pub provider_thread_id: Option<String>,
    /// Gmail label ids.
    pub labels: Vec<String>,
    /// The RFC 822 bytes.
    pub raw: Vec<u8>,
}

/// What [`ingest`] stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingested {
    pub id: i64,
    pub thread_id: i64,
    /// `false` when the `(source, provider_message_id)` row already existed and was updated.
    pub inserted: bool,
    pub verdict: VerdictKind,
    /// The `classify_email` run this ingest queued, if any.
    pub classify_run_id: Option<i64>,
}

#[derive(Debug)]
pub enum IngestError {
    /// The bytes aren't a message; nothing was written.
    Parse(ParseError),
    Db(sqlx::Error),
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => err.fmt(f),
            Self::Db(err) => write!(f, "cannot store the email: {err}"),
        }
    }
}

impl std::error::Error for IngestError {}

impl From<sqlx::Error> for IngestError {
    fn from(err: sqlx::Error) -> Self {
        Self::Db(err)
    }
}

/// A row of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct EmailSummary {
    pub id: i64,
    pub thread_id: i64,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub subject: String,
    pub sent_at: Option<DateTime<Utc>>,
    pub verdict: VerdictKind,
    pub ats: Option<Ats>,
}

/// Parses, normalizes and scores `new` (outside the transaction), then in one transaction:
/// takes the thread advisory lock, keeps the existing row's thread or assigns one,
/// upserts the row on `(source, provider_message_id)`, moves the thread's
/// `last_message_at` and, for a `candidate`, queues a `classify_email` run unless the
/// email already has a classification row for its current input key (see
/// [`classify_email::queue`]). Ingesting the same message again updates the derived
/// columns, `raw` and `labels` in place; `id`, `thread_id` and `created_at` stay. After the
/// commit a queued run is published and the worker woken.
pub async fn ingest(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    new: NewEmail,
    known: &KnownCompanies,
) -> Result<Ingested, IngestError> {
    let (ingested, run) = store(pool, new, known, Some(bus)).await?;
    if let Some(run) = run {
        bus.publish(Event::RunUpdated { run });
        wake.0.notify_one();
    }
    Ok(ingested)
}

/// [`ingest`] without queuing a classification run, for eval setup: a seeded case must not
/// start a second run that would be charged to it.
pub(crate) async fn ingest_unqueued(
    pool: &PgPool,
    new: NewEmail,
    known: &KnownCompanies,
) -> Result<Ingested, IngestError> {
    store(pool, new, known, None)
        .await
        .map(|(ingested, _)| ingested)
}

/// The work of [`ingest`]; queues a classification run only with `queue`.
async fn store(
    pool: &PgPool,
    new: NewEmail,
    known: &KnownCompanies,
    queue: Option<&EventBus>,
) -> Result<(Ingested, Option<Run>), IngestError> {
    let parsed = mime::parse(&new.raw).map_err(IngestError::Parse)?;
    let normalized = normalize::normalize(&parsed);
    let verdict = heuristics::score(&HeuristicInput::new(&parsed, &normalized), known);
    let subject = thread::normalize_subject(&parsed.subject);
    let headers = json!(
        parsed
            .headers
            .iter()
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect::<Vec<_>>()
    );
    let addresses = |list: &[mime::Address]| -> Vec<String> {
        list.iter().map(|a| a.address.clone()).collect()
    };

    let mut tx = pool.begin().await?;
    // Concurrent ingests could otherwise each start a thread for the same conversation.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('waypoint.email_threads'))")
        .execute(&mut *tx)
        .await?;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT thread_id FROM emails WHERE source = $1 AND provider_message_id = $2",
    )
    .bind(new.source.as_str())
    .bind(&new.provider_message_id)
    .fetch_optional(&mut *tx)
    .await?;
    let thread_id = match existing {
        Some(id) => id,
        None => {
            thread::assign_thread(
                &mut tx,
                &parsed,
                &subject,
                new.provider_thread_id.as_deref(),
            )
            .await?
        }
    };

    let row = sqlx::query(
        "INSERT INTO emails (source, provider_message_id, thread_id, rfc_message_id, \
           in_reply_to, references_ids, labels, from_address, from_name, to_addresses, \
           cc_addresses, subject, sent_at, headers, raw, body_text, body_source, signature, \
           verdict, verdict_reasons, ats, normalizer_version, heuristics_version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
           $18, $19, $20, $21, $22, $23) \
         ON CONFLICT (source, provider_message_id) DO UPDATE SET \
           rfc_message_id = EXCLUDED.rfc_message_id, in_reply_to = EXCLUDED.in_reply_to, \
           references_ids = EXCLUDED.references_ids, labels = EXCLUDED.labels, \
           from_address = EXCLUDED.from_address, from_name = EXCLUDED.from_name, \
           to_addresses = EXCLUDED.to_addresses, cc_addresses = EXCLUDED.cc_addresses, \
           subject = EXCLUDED.subject, sent_at = EXCLUDED.sent_at, \
           headers = EXCLUDED.headers, raw = EXCLUDED.raw, body_text = EXCLUDED.body_text, \
           body_source = EXCLUDED.body_source, signature = EXCLUDED.signature, \
           verdict = EXCLUDED.verdict, verdict_reasons = EXCLUDED.verdict_reasons, \
           ats = EXCLUDED.ats, normalizer_version = EXCLUDED.normalizer_version, \
           heuristics_version = EXCLUDED.heuristics_version, updated_at = now() \
         RETURNING id, thread_id, (xmax = 0) AS inserted",
    )
    .bind(new.source.as_str())
    .bind(&new.provider_message_id)
    .bind(thread_id)
    .bind(&parsed.rfc_message_id)
    .bind(&parsed.in_reply_to)
    .bind(&parsed.references)
    .bind(&new.labels)
    .bind(parsed.from.as_ref().map(|a| a.address.as_str()))
    .bind(parsed.from.as_ref().and_then(|a| a.name.as_deref()))
    .bind(addresses(&parsed.to))
    .bind(addresses(&parsed.cc))
    .bind(&parsed.subject)
    .bind(parsed.sent_at)
    .bind(&headers)
    .bind(&new.raw)
    .bind(&normalized.body_text)
    .bind(normalized.body_source.as_str())
    .bind(&normalized.signature)
    .bind(verdict.verdict.as_str())
    .bind(json!(verdict.reasons))
    .bind(verdict.ats.map(Ats::as_str))
    .bind(NORMALIZER_VERSION)
    .bind(HEURISTICS_VERSION)
    .fetch_one(&mut *tx)
    .await?;
    let id: i64 = row.get("id");
    let thread_id: i64 = row.get("thread_id");
    let inserted: bool = row.get("inserted");

    thread::touch(&mut tx, thread_id, parsed.sent_at).await?;

    let mut run = None;
    if let Some(bus) = queue
        && verdict.verdict == VerdictKind::Candidate
    {
        let key = classify_email::input_key(&KeyInput {
            subject: &parsed.subject,
            from_address: parsed.from.as_ref().map(|a| a.address.as_str()),
            from_name: parsed.from.as_ref().and_then(|a| a.name.as_deref()),
            sent_at: parsed.sent_at,
            body_text: &normalized.body_text,
            signature: normalized.signature.as_deref(),
            ats: verdict.ats.map(Ats::as_str),
        });
        run = classify_email::queue(&mut tx, bus, id, &key).await?;
    }
    tx.commit().await?;

    let ingested = Ingested {
        id,
        thread_id,
        inserted,
        verdict: verdict.verdict,
        classify_run_id: run.as_ref().map(|run| run.id),
    };
    Ok((ingested, run))
}

/// Stored emails, newest first (by `sent_at`, then id), optionally only one verdict.
pub async fn list(
    pool: &PgPool,
    verdict: Option<VerdictKind>,
    limit: i64,
) -> Result<Vec<EmailSummary>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, thread_id, from_address, from_name, subject, sent_at, verdict, ats \
         FROM emails WHERE ($1::text IS NULL OR verdict = $1) \
         ORDER BY sent_at DESC NULLS LAST, id DESC LIMIT $2",
    )
    .bind(verdict.map(VerdictKind::as_str))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let verdict: String = row.try_get("verdict")?;
            let ats: Option<String> = row.try_get("ats")?;
            Ok(EmailSummary {
                id: row.try_get("id")?,
                thread_id: row.try_get("thread_id")?,
                from_address: row.try_get("from_address")?,
                from_name: row.try_get("from_name")?,
                subject: row.try_get("subject")?,
                sent_at: row.try_get("sent_at")?,
                verdict: VerdictKind::parse(&verdict).ok_or_else(|| {
                    sqlx::Error::Decode(format!("unknown verdict {verdict:?}").into())
                })?,
                ats: ats.as_deref().and_then(Ats::parse),
            })
        })
        .collect()
}

/// The company keys of every tracked application, for the `known_company` rule. Load it
/// once per import.
pub async fn known_companies(pool: &PgPool) -> Result<KnownCompanies, sqlx::Error> {
    let companies: Vec<String> = sqlx::query_scalar("SELECT DISTINCT company FROM applications")
        .fetch_all(pool)
        .await?;
    Ok(KnownCompanies::new(companies.iter().map(String::as_str)))
}
