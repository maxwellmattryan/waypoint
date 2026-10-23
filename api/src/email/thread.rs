//! Grouping messages into threads: subject normalization (pure) and thread assignment
//! (in the ingest transaction).

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};

use super::mime::ParsedEmail;

/// Reply and forward prefixes, lowercased: English, German, Scandinavian, Dutch, Italian
/// and French clients.
const PREFIXES: [&str; 10] = [
    "re", "fw", "fwd", "aw", "wg", "sv", "vs", "antw", "ri", "tr",
];
/// Tags some mail gateways put in front of the subject.
const TAGS: [&str; 2] = ["[external]", "[ext]"];
/// How far back the reply-subject fallback looks for a thread's last message.
const SUBJECT_WINDOW_DAYS: i64 = 90;

/// A subject with its reply prefixes and gateway tags removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    /// For display: prefixes and tags removed, whitespace collapsed.
    pub display: String,
    /// `display` lowercased, for matching.
    pub key: String,
    /// Whether a reply or forward prefix was removed.
    pub was_reply: bool,
}

/// Removes repeated leading `Re`/`Fw`/`Fwd`/`AW`/`WG`/`SV`/`VS`/`Antw`/`RI`/`TR` prefixes
/// (any case, with an optional `[n]` or `(n)` count) and `[External]`/`[EXT]` tags, and
/// collapses whitespace.
pub fn normalize_subject(s: &str) -> Subject {
    let mut rest = s.trim();
    let mut was_reply = false;
    loop {
        if let Some(after) = strip_tag(rest) {
            rest = after;
        } else if let Some(after) = strip_prefix(rest) {
            rest = after;
            was_reply = true;
        } else {
            break;
        }
    }
    let display = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    Subject {
        key: display.to_lowercase(),
        display,
        was_reply,
    }
}

fn strip_tag(s: &str) -> Option<&str> {
    TAGS.iter().find_map(|tag| {
        let head = s.get(..tag.len())?;
        head.eq_ignore_ascii_case(tag)
            .then(|| s[tag.len()..].trim_start())
    })
}

/// `Re: x`, `RE[2]: x`, `Fwd (3): x` → `x`.
fn strip_prefix(s: &str) -> Option<&str> {
    let word_len = s.find(|c: char| !c.is_ascii_alphabetic())?;
    let word = s[..word_len].to_ascii_lowercase();
    if !PREFIXES.contains(&word.as_str()) {
        return None;
    }
    let mut rest = s[word_len..].trim_start();
    for (open, close) in [('[', ']'), ('(', ')')] {
        if let Some(inner) = rest.strip_prefix(open) {
            let end = inner.find(close)?;
            if end == 0 || !inner[..end].chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            rest = inner[end + 1..].trim_start();
            break;
        }
    }
    let rest = rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))?;
    Some(rest.trim_start())
}

/// Every participant address of a message (from, to, cc), lowercased.
pub fn participants(parsed: &ParsedEmail) -> Vec<String> {
    let mut all: Vec<String> = parsed
        .from
        .iter()
        .chain(&parsed.to)
        .chain(&parsed.cc)
        .map(|a| a.address.clone())
        .collect();
    all.sort();
    all.dedup();
    all
}

/// The thread a new message joins, or a new thread. In order:
///
/// 1. The thread with this Gmail thread id.
/// 2. The thread of an existing email whose `rfc_message_id` is in this message's
///    `In-Reply-To`/`References`, or whose `references_ids` contains this message's id.
/// 3. For a message that looks like a reply (a removed prefix, or reply headers), the most
///    recent thread with the same subject key that shares a participant and whose
///    `last_message_at` is within 90 days of this message's `sent_at`.
///
/// A new thread takes the Gmail thread id when there is one. A thread found by headers or
/// subject that has no Gmail thread id yet takes this message's.
pub async fn assign_thread(
    conn: &mut PgConnection,
    parsed: &ParsedEmail,
    subject: &Subject,
    provider_thread_id: Option<&str>,
) -> Result<i64, sqlx::Error> {
    if let Some(gmail) = provider_thread_id {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT id FROM email_threads WHERE provider_thread_id = $1")
                .bind(gmail)
                .fetch_optional(&mut *conn)
                .await?;
        if let Some(id) = found {
            return Ok(id);
        }
    }

    let found = by_headers(conn, parsed).await?;
    let found = match found {
        Some(id) => Some(id),
        None => by_subject(conn, parsed, subject).await?,
    };
    if let Some(id) = found {
        if let Some(gmail) = provider_thread_id {
            sqlx::query(
                "UPDATE email_threads SET provider_thread_id = $2, updated_at = now() \
                 WHERE id = $1 AND provider_thread_id IS NULL",
            )
            .bind(id)
            .bind(gmail)
            .execute(&mut *conn)
            .await?;
        }
        return Ok(id);
    }

    sqlx::query_scalar(
        "INSERT INTO email_threads (provider_thread_id, subject, subject_key, last_message_at) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(provider_thread_id)
    .bind(&subject.display)
    .bind(&subject.key)
    .bind(parsed.sent_at)
    .fetch_one(&mut *conn)
    .await
}

async fn by_headers(
    conn: &mut PgConnection,
    parsed: &ParsedEmail,
) -> Result<Option<i64>, sqlx::Error> {
    let mut linked: Vec<String> = parsed.in_reply_to.clone();
    linked.extend(parsed.references.iter().cloned());
    if linked.is_empty() && parsed.rfc_message_id.is_none() {
        return Ok(None);
    }
    sqlx::query_scalar(
        "SELECT thread_id FROM emails \
         WHERE rfc_message_id = ANY($1) OR ($2::text IS NOT NULL AND references_ids @> ARRAY[$2]) \
         ORDER BY sent_at DESC NULLS LAST, id DESC LIMIT 1",
    )
    .bind(&linked)
    .bind(parsed.rfc_message_id.as_deref())
    .fetch_optional(&mut *conn)
    .await
}

async fn by_subject(
    conn: &mut PgConnection,
    parsed: &ParsedEmail,
    subject: &Subject,
) -> Result<Option<i64>, sqlx::Error> {
    let looks_like_reply =
        subject.was_reply || !parsed.in_reply_to.is_empty() || !parsed.references.is_empty();
    let (true, Some(sent_at)) = (looks_like_reply, parsed.sent_at) else {
        return Ok(None);
    };
    if subject.key.is_empty() {
        return Ok(None);
    }
    let window = chrono::Duration::days(SUBJECT_WINDOW_DAYS);
    let row = sqlx::query(
        "SELECT t.id FROM email_threads t \
         WHERE t.subject_key = $1 \
           AND t.last_message_at BETWEEN $2 AND $3 \
           AND EXISTS ( \
             SELECT 1 FROM emails e WHERE e.thread_id = t.id AND ( \
               e.from_address = ANY($4) OR e.to_addresses && $4 OR e.cc_addresses && $4)) \
         ORDER BY t.last_message_at DESC, t.id DESC LIMIT 1",
    )
    .bind(&subject.key)
    .bind(sent_at - window)
    .bind(sent_at + window)
    .bind(participants(parsed))
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| r.get::<i64, _>(0)))
}

/// `last_message_at` for a thread after a message sent at `sent_at` joined it.
pub async fn touch(
    conn: &mut PgConnection,
    thread_id: i64,
    sent_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE email_threads SET last_message_at = GREATEST(last_message_at, $2), \
         updated_at = now() WHERE id = $1",
    )
    .bind(thread_id)
    .bind(sent_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalize_subject() {
        // (input, display, was_reply)
        let cases = [
            ("Interview with Acme", "Interview with Acme", false),
            ("Re: Interview", "Interview", true),
            ("RE: Re: FW: Interview", "Interview", true),
            ("Fwd: Interview", "Interview", true),
            ("AW: WG: Interview", "Interview", true),
            ("SV: VS: Antw: Interview", "Interview", true),
            ("RI: TR: Interview", "Interview", true),
            ("Re[2]: Interview", "Interview", true),
            ("RE (3): Interview", "Interview", true),
            ("[External] Re: Interview", "Interview", true),
            ("[EXT] Interview", "Interview", false),
            ("Re:   Interview   with\tAcme ", "Interview with Acme", true),
            ("Reminder: interview", "Reminder: interview", false),
            ("Re [x]: Interview", "Re [x]: Interview", false),
            ("", "", false),
        ];
        for (input, display, was_reply) in cases {
            let got = super::normalize_subject(input);
            assert_eq!(got.display, display, "{input}");
            assert_eq!(got.key, display.to_lowercase(), "{input}");
            assert_eq!(got.was_reply, was_reply, "{input}");
        }
    }
}
