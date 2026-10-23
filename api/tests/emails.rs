//! The email store: schema constraints, threading through `email::ingest`, and the
//! `POST /dev/emails/import-fixtures` route over `api/fixtures/emails/`.

mod common;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use waypoint_api::email::heuristics::KnownCompanies;
use waypoint_api::email::{self, Ingested, NewEmail, Source, VerdictKind};

const IMPORT: &str = "/dev/emails/import-fixtures";
const MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/emails/manifest.json");

/// One raw message with the given headers and a short text body.
fn raw(headers: &[(&str, &str)]) -> Vec<u8> {
    let mut out = String::new();
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("Content-Type: text/plain; charset=utf-8\r\n\r\nHello.\r\n");
    out.into_bytes()
}

async fn ingest(pool: &PgPool, id: &str, gmail_thread: Option<&str>, raw: Vec<u8>) -> Ingested {
    email::ingest(
        pool,
        NewEmail {
            source: Source::Gmail,
            provider_message_id: id.into(),
            provider_thread_id: gmail_thread.map(Into::into),
            labels: vec!["INBOX".into()],
            raw,
        },
        &KnownCompanies::default(),
    )
    .await
    .expect("ingest")
}

async fn client(pool: PgPool) -> (Client, common::Fake) {
    let fake = common::fake("success", Duration::from_secs(10));
    let rocket = common::app(pool, fake.runner.clone(), Default::default());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

async fn import(client: &Client) -> Value {
    let response = client
        .post(IMPORT)
        .header(Header::new("Host", common::HOST))
        .header(Header::new("X-Waypoint-Dev", "1"))
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok);
    response.into_json().await.expect("json body")
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

#[derive(Deserialize)]
struct Manifest {
    synthetic: bool,
    emails: Vec<Expectation>,
}

#[derive(Deserialize)]
struct Expectation {
    file: String,
    verdict: String,
    ats: Option<String>,
    reasons_include: Vec<String>,
    body_contains: Vec<String>,
    body_excludes: Vec<String>,
    thread: String,
}

fn manifest() -> Manifest {
    let text = std::fs::read_to_string(MANIFEST).expect("read manifest");
    serde_json::from_str(&text).expect("valid manifest")
}

/// Inserts one `emails` row with `column` set to `value` and every other column valid.
async fn insert_email_with(pool: &PgPool, column: &str, value: &str) -> Result<(), sqlx::Error> {
    let thread: i64 = sqlx::query_scalar(
        "INSERT INTO email_threads (subject, subject_key) VALUES ('s', 's') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert thread");
    let mut values = [
        ("source", "fixture"),
        ("verdict", "skip"),
        ("ats", "greenhouse"),
        ("body_source", "plain"),
    ];
    for (name, v) in values.iter_mut() {
        if *name == column {
            *v = value;
        }
    }
    sqlx::query(
        "INSERT INTO emails (source, provider_message_id, thread_id, headers, raw, body_text, \
         body_source, verdict, verdict_reasons, ats, normalizer_version, heuristics_version) \
         VALUES ($1, gen_random_uuid()::text, $2, '[]', '\\x00', '', $3, $4, '[]', $5, 1, 1)",
    )
    .bind(values[0].1)
    .bind(thread)
    .bind(values[3].1)
    .bind(values[1].1)
    .bind(values[2].1)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test]
async fn check_constraints_refuse_unknown_values(pool: PgPool) {
    insert_email_with(&pool, "source", "gmail")
        .await
        .expect("a valid row is accepted");

    for column in ["source", "verdict", "ats", "body_source"] {
        let err = insert_email_with(&pool, column, "unknown")
            .await
            .expect_err(column);
        let code = err
            .as_database_error()
            .and_then(|db| db.code())
            .map(|c| c.to_string());
        assert_eq!(code.as_deref(), Some("23514"), "{column}: {err}");
    }

    let reasons = sqlx::query(
        "INSERT INTO emails (source, provider_message_id, thread_id, headers, raw, body_text, \
         body_source, verdict, verdict_reasons, normalizer_version, heuristics_version) \
         SELECT 'fixture', 'obj', id, '[]', '\\x00', '', 'none', 'skip', '{}', 1, 1 \
         FROM email_threads LIMIT 1",
    )
    .execute(&pool)
    .await
    .expect_err("verdict_reasons must be an array");
    assert_eq!(
        reasons
            .as_database_error()
            .and_then(|db| db.code())
            .as_deref(),
        Some("23514")
    );

    let duplicate = "INSERT INTO emails (source, provider_message_id, thread_id, headers, raw, \
                     body_text, body_source, verdict, verdict_reasons, normalizer_version, \
                     heuristics_version) \
                     SELECT 'fixture', 'same', id, '[]', '\\x00', '', 'none', 'skip', '[]', 1, 1 \
                     FROM email_threads LIMIT 1";
    sqlx::query(duplicate)
        .execute(&pool)
        .await
        .expect("first copy");
    let err = sqlx::query(duplicate)
        .execute(&pool)
        .await
        .expect_err("(source, provider_message_id) is unique");
    assert_eq!(
        err.as_database_error().and_then(|db| db.code()).as_deref(),
        Some("23505")
    );

    let thread = "INSERT INTO email_threads (provider_thread_id, subject, subject_key) \
                  VALUES ('t1', 's', 's')";
    sqlx::query(thread)
        .execute(&pool)
        .await
        .expect("first thread");
    let err = sqlx::query(thread)
        .execute(&pool)
        .await
        .expect_err("provider_thread_id is unique");
    assert_eq!(
        err.as_database_error().and_then(|db| db.code()).as_deref(),
        Some("23505")
    );
}

#[sqlx::test]
async fn threads_group_by_gmail_id_headers_and_reply_subject(pool: PgPool) {
    // 1. The same Gmail thread id groups messages whose headers are unrelated.
    let a = ingest(
        &pool,
        "g-a",
        Some("gmail-thread-1"),
        raw(&[
            ("From", "ana@acme.example"),
            ("To", "me@home.example"),
            ("Subject", "Coffee chat"),
            ("Date", "Mon, 05 Oct 2026 10:00:00 +0000"),
            ("Message-ID", "<a@acme.example>"),
        ]),
    )
    .await;
    let b = ingest(
        &pool,
        "g-b",
        Some("gmail-thread-1"),
        raw(&[
            ("From", "bo@other.example"),
            ("To", "me@home.example"),
            ("Subject", "Something else entirely"),
            ("Date", "Tue, 06 Oct 2026 10:00:00 +0000"),
            ("Message-ID", "<b@other.example>"),
        ]),
    )
    .await;
    assert!(a.inserted && b.inserted);
    assert_eq!(a.thread_id, b.thread_id, "same Gmail thread id");

    // 2. A reply that arrives before its parent ends up in the parent's thread.
    let child = ingest(
        &pool,
        "child",
        None,
        raw(&[
            ("From", "me@home.example"),
            ("To", "cy@initech.example"),
            ("Subject", "Re: Next week"),
            ("Date", "Thu, 08 Oct 2026 10:00:00 +0000"),
            ("Message-ID", "<child@home.example>"),
            ("In-Reply-To", "<parent@initech.example>"),
            ("References", "<parent@initech.example>"),
        ]),
    )
    .await;
    let parent = ingest(
        &pool,
        "parent",
        None,
        raw(&[
            ("From", "cy@initech.example"),
            ("To", "me@home.example"),
            ("Subject", "Onsite logistics"),
            ("Date", "Wed, 07 Oct 2026 10:00:00 +0000"),
            ("Message-ID", "<parent@initech.example>"),
        ]),
    )
    .await;
    assert_eq!(parent.thread_id, child.thread_id, "out-of-order parent");
    assert_ne!(parent.thread_id, a.thread_id);
    let last: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_message_at FROM email_threads WHERE id = $1")
            .bind(parent.thread_id)
            .fetch_one(&pool)
            .await
            .expect("thread");
    assert_eq!(last.to_rfc3339(), "2026-10-08T10:00:00+00:00");

    // 3. A `Re:` with no reference headers joins the thread sharing a participant and
    //    subject; one with no shared participant, or past 90 days, doesn't.
    let outreach = ingest(
        &pool,
        "outreach",
        None,
        raw(&[
            ("From", "dana@soylent.example"),
            ("To", "me@home.example"),
            ("Subject", "Staff role"),
            ("Date", "Thu, 01 Oct 2026 10:00:00 +0000"),
            ("Message-ID", "<o@soylent.example>"),
        ]),
    )
    .await;
    let reply = ingest(
        &pool,
        "reply",
        None,
        raw(&[
            ("From", "me@home.example"),
            ("To", "Dana <DANA@soylent.example>"),
            ("Subject", "RE: Re: Staff  role"),
            ("Date", "Fri, 02 Oct 2026 10:00:00 +0000"),
            ("Message-ID", "<r@home.example>"),
        ]),
    )
    .await;
    assert_eq!(reply.thread_id, outreach.thread_id, "reply subject");
    let stranger = ingest(
        &pool,
        "stranger",
        None,
        raw(&[
            ("From", "zed@elsewhere.example"),
            ("To", "kim@elsewhere.example"),
            ("Subject", "Re: Staff role"),
            ("Date", "Fri, 02 Oct 2026 11:00:00 +0000"),
            ("Message-ID", "<s@elsewhere.example>"),
        ]),
    )
    .await;
    assert_ne!(
        stranger.thread_id, outreach.thread_id,
        "no shared participant"
    );
    let late = ingest(
        &pool,
        "late",
        None,
        raw(&[
            ("From", "me@home.example"),
            ("To", "dana@soylent.example"),
            ("Subject", "Re: Staff role"),
            ("Date", "Mon, 01 Feb 2027 10:00:00 +0000"),
            ("Message-ID", "<late@home.example>"),
        ]),
    )
    .await;
    assert_ne!(late.thread_id, outreach.thread_id, "outside 90 days");

    // 4. Two non-reply ATS emails with the same subject and sender stay apart.
    let ats = |id: &'static str, date: &'static str| {
        raw(&[
            ("From", "no-reply@us.greenhouse-mail.io"),
            ("To", "me@home.example"),
            ("Subject", "Thank you for applying!"),
            ("Date", date),
            ("Message-ID", id),
        ])
    };
    let first = ingest(
        &pool,
        "ats-1",
        None,
        ats("<1@gh.example>", "Mon, 05 Oct 2026 09:00:00 +0000"),
    )
    .await;
    let second = ingest(
        &pool,
        "ats-2",
        None,
        ats("<2@gh.example>", "Tue, 06 Oct 2026 09:00:00 +0000"),
    )
    .await;
    assert_ne!(first.thread_id, second.thread_id, "separate ATS mail");
    assert_eq!(first.verdict, VerdictKind::Candidate);

    // Ingesting the same message again updates it in place.
    let again = ingest(
        &pool,
        "ats-1",
        None,
        ats("<1@gh.example>", "Mon, 05 Oct 2026 09:00:00 +0000"),
    )
    .await;
    assert_eq!((again.id, again.thread_id), (first.id, first.thread_id));
    assert!(!again.inserted);
    assert_eq!(count(&pool, "emails").await, 10);

    let listed = email::list(&pool, Some(VerdictKind::Candidate), 100)
        .await
        .expect("list");
    assert!(listed.iter().all(|e| e.verdict == VerdictKind::Candidate));
    assert!(listed.iter().any(|e| e.id == first.id));
}

#[sqlx::test]
async fn ingest_refuses_unparseable_bytes(pool: PgPool) {
    let err = email::ingest(
        &pool,
        NewEmail {
            source: Source::Fixture,
            provider_message_id: "empty".into(),
            provider_thread_id: None,
            labels: Vec::new(),
            raw: Vec::new(),
        },
        &KnownCompanies::default(),
    )
    .await
    .expect_err("empty input");
    assert!(matches!(err, email::IngestError::Parse(_)), "{err}");
    assert_eq!(count(&pool, "emails").await, 0);
    assert_eq!(count(&pool, "email_threads").await, 0);
}

#[sqlx::test]
async fn fixture_import_matches_manifest(pool: PgPool) {
    let manifest = manifest();
    assert!(manifest.synthetic);
    let (client, _fake) = client(pool.clone()).await;
    let body = import(&client).await;

    let files = manifest.emails.len();
    assert!((18..=25).contains(&files), "{files} fixtures");
    let candidates = manifest
        .emails
        .iter()
        .filter(|e| e.verdict == "candidate")
        .count();
    let labels: HashSet<&str> = manifest.emails.iter().map(|e| e.thread.as_str()).collect();
    assert_eq!(body["files"], files, "{body}");
    assert_eq!(body["inserted"], files, "{body}");
    assert_eq!(body["updated"], 0, "{body}");
    assert_eq!(body["candidates"], candidates, "{body}");
    assert_eq!(body["skipped"], files - candidates, "{body}");
    assert_eq!(body["threads"], labels.len(), "{body}");
    assert_eq!(body["failed"], serde_json::json!([]), "{body}");

    let mut thread_of_label: HashMap<String, i64> = HashMap::new();
    let mut label_of_thread: HashMap<i64, String> = HashMap::new();
    for want in &manifest.emails {
        let stem = want.file.trim_end_matches(".eml");
        let (thread_id, verdict, ats, reasons, body_text): (
            i64,
            String,
            Option<String>,
            Value,
            String,
        ) = sqlx::query_as(
            "SELECT thread_id, verdict, ats, verdict_reasons, body_text FROM emails \
             WHERE source = 'fixture' AND provider_message_id = $1",
        )
        .bind(stem)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|err| panic!("{}: {err}", want.file));
        let file = &want.file;
        assert_eq!(verdict, want.verdict, "{file}: {reasons}");
        assert_eq!(ats, want.ats, "{file}");
        let reasons: Vec<String> = serde_json::from_value(reasons).expect("reasons array");
        for code in &want.reasons_include {
            assert!(reasons.contains(code), "{file}: {code} not in {reasons:?}");
        }
        for text in &want.body_contains {
            assert!(
                body_text.contains(text),
                "{file}: {text:?} not in {body_text:?}"
            );
        }
        for text in &want.body_excludes {
            assert!(
                !body_text.contains(text),
                "{file}: {text:?} in {body_text:?}"
            );
        }
        let by_label = *thread_of_label
            .entry(want.thread.clone())
            .or_insert(thread_id);
        assert_eq!(by_label, thread_id, "{file}: same label, same thread");
        let by_thread = label_of_thread
            .entry(thread_id)
            .or_insert(want.thread.clone());
        assert_eq!(
            *by_thread, want.thread,
            "{file}: different labels, different threads"
        );
    }

    // Nothing here calls a model or queues work.
    assert_eq!(count(&pool, "claude_calls").await, 0);
    assert_eq!(count(&pool, "jobs").await, 0);
}

#[sqlx::test]
async fn fixture_import_is_idempotent(pool: PgPool) {
    let (client, _fake) = client(pool.clone()).await;
    let snapshot = || async {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT provider_message_id, id, thread_id FROM emails WHERE source = 'fixture'",
        )
        .fetch_all(&pool)
        .await
        .expect("rows");
        rows.into_iter()
            .map(|(name, id, thread)| (name, (id, thread)))
            .collect::<BTreeMap<_, _>>()
    };

    let first = import(&client).await;
    let before = snapshot().await;
    let threads_before = count(&pool, "email_threads").await;
    let second = import(&client).await;

    let files = first["files"].as_u64().expect("files");
    assert_eq!(second["files"], files);
    assert_eq!(second["inserted"], 0, "{second}");
    assert_eq!(second["updated"], files, "{second}");
    assert_eq!(second["failed"], serde_json::json!([]));
    assert_eq!(second["threads"], first["threads"]);
    assert_eq!(snapshot().await, before);
    assert_eq!(count(&pool, "emails").await, files as i64);
    assert_eq!(count(&pool, "email_threads").await, threads_before);
}

#[sqlx::test]
async fn import_refuses_unmarked_cross_site_and_form_requests(pool: PgPool) {
    let (client, _fake) = client(pool.clone()).await;
    let host = || Header::new("Host", common::HOST);
    let dev = || Header::new("X-Waypoint-Dev", "1");

    let refused = [
        ("no dev header", client.post(IMPORT).header(host())),
        (
            "form without the header",
            client
                .post(IMPORT)
                .header(host())
                .header(ContentType::Form)
                .body("a=1"),
        ),
        (
            "cross-site with the header",
            client
                .post(IMPORT)
                .header(host())
                .header(dev())
                .header(Header::new("Sec-Fetch-Site", "cross-site")),
        ),
        (
            "non-local host",
            client
                .post(IMPORT)
                .header(Header::new("Host", "evil.example"))
                .header(dev()),
        ),
    ];
    for (name, request) in refused {
        let response = request.dispatch().await;
        assert_eq!(response.status(), Status::Forbidden, "{name}");
    }
    assert_eq!(count(&pool, "emails").await, 0);
    assert_eq!(count(&pool, "email_threads").await, 0);
}
