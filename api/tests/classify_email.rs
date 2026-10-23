//! The `classify_email` workflow end to end: the `email_classifications` table, the
//! worker running the `classify` step against `fake-claude.sh`, and the runs ingest
//! queues.

mod common;

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::events::EventBus;
use waypoint_api::jobs::Wake;
use waypoint_api::runs;
use waypoint_api::workflows::{self, classify_email};

const TIMEOUT: Duration = Duration::from_secs(20);

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The API with the production runtime and a runner playing `mode`. Keep the `Fake` alive
/// as long as the client.
async fn app(pool: &PgPool, mode: &str) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(30));
    let rocket = common::app(pool.clone(), fake.runner.clone(), common::fast_config());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// Waits until run `id` is `status`, and returns its output.
async fn until(pool: &PgPool, id: i64, status: &str) -> Value {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let (current, output): (String, Option<Value>) =
            sqlx::query_as("SELECT status, output FROM agent_runs WHERE id = $1")
                .bind(id)
                .fetch_one(pool)
                .await
                .expect("read the run");
        if current == status {
            return output.unwrap_or(Value::Null);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "run {id} is {current}, never {status}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"))
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant")
        .with_timezone(&Utc)
}

async fn insert_thread(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO email_threads (subject, subject_key) VALUES ('Staff role', 'staff role') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert a thread")
}

/// A stored candidate email from Dana at Soylent, without a classification.
struct Seeded {
    id: i64,
    subject: &'static str,
    sent_at: DateTime<Utc>,
    body: &'static str,
}

impl Seeded {
    /// Its current input key.
    fn key(&self) -> String {
        classify_email::input_key(&classify_email::KeyInput {
            subject: self.subject,
            from_address: Some("dana@soylent.example"),
            from_name: Some("Dana Lee"),
            sent_at: Some(self.sent_at),
            body_text: self.body,
            signature: None,
            ats: None,
        })
    }
}

async fn insert_email(
    pool: &PgPool,
    thread_id: i64,
    provider_id: &str,
    subject: &'static str,
    sent_at: &str,
    body: &'static str,
) -> Seeded {
    let sent_at = instant(sent_at);
    let headers = json!([{"name": "Date", "value": sent_at.to_rfc2822()}]);
    let id = sqlx::query_scalar(
        "INSERT INTO emails (source, provider_message_id, thread_id, from_address, from_name, \
         subject, sent_at, headers, raw, body_text, body_source, verdict, verdict_reasons, \
         normalizer_version, heuristics_version) \
         VALUES ('fixture', $1, $2, 'dana@soylent.example', 'Dana Lee', $3, $4, $5, '\\x00', \
         $6, 'plain', 'candidate', '[]', 1, 1) RETURNING id",
    )
    .bind(provider_id)
    .bind(thread_id)
    .bind(subject)
    .bind(sent_at)
    .bind(headers)
    .bind(body)
    .fetch_one(pool)
    .await
    .expect("insert an email");
    Seeded {
        id,
        subject,
        sent_at,
        body,
    }
}

/// The email the worker tests classify, after an earlier message of its thread.
async fn seed_thread(pool: &PgPool) -> Seeded {
    let thread = insert_thread(pool).await;
    insert_email(
        pool,
        thread,
        "earlier",
        "Staff engineer role at Soylent",
        "2026-10-20T16:00:00Z",
        "I came across your work on distributed systems EARLIERMARKER",
    )
    .await;
    insert_email(
        pool,
        thread,
        "latest",
        "Times for a recruiter call",
        "2026-10-22T16:00:00Z",
        "Would Tue Oct 27 at 2pm PT or Tue Nov 3 at 2pm PT work for a recruiter call?",
    )
    .await
}

/// Starts a `classify_email` run with `input` and returns its id.
async fn start(pool: &PgPool, input: Value) -> i64 {
    runs::start(
        pool,
        &EventBus::default(),
        &Wake::default(),
        &workflows::runtime(),
        classify_email::WORKFLOW,
        input,
    )
    .await
    .expect("start a run")
    .id
}

/// Inserts a classification row with extra `columns` set to the SQL `values`.
async fn insert_row(
    pool: &PgPool,
    email_id: i64,
    key: &str,
    columns: &str,
    values: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "INSERT INTO email_classifications (email_id, input_key, classifier_version{columns}) \
         VALUES ($1, $2, 1{values}) RETURNING id"
    ))
    .bind(email_id)
    .bind(key)
    .fetch_one(pool)
    .await
}

fn code(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|db| db.code())
        .map(|code| code.to_string())
}

#[sqlx::test]
async fn classification_table_enforces_constraints(pool: PgPool) {
    let thread = insert_thread(&pool).await;
    let email = insert_email(&pool, thread, "a", "s", "2026-10-22T16:00:00Z", "b").await;
    insert_row(&pool, email.id, "pending", "", "")
        .await
        .expect("a pending row");
    insert_row(
        &pool,
        email.id,
        "classified",
        ", status, class, confidence, proposed_times",
        ", 'classified', 'offer', 0.5, '[{\"text\": \"t\"}]'",
    )
    .await
    .expect("a classified row");

    let refused = [
        ("unknown status", ", status", ", 'done'"),
        ("unknown class", ", class", ", 'maybe'"),
        ("confidence above 1", ", confidence", ", 1.5"),
        ("negative confidence", ", confidence", ", -0.1"),
        (
            "classified without a class",
            ", status, confidence",
            ", 'classified', 0.5",
        ),
        (
            "classified without a confidence",
            ", status, class",
            ", 'classified', 'offer'",
        ),
        ("object proposed_times", ", proposed_times", ", '{}'"),
        ("object issues", ", issues", ", '{}'"),
        ("string invalid_reasons", ", invalid_reasons", ", '\"x\"'"),
    ];
    for (index, (name, columns, values)) in refused.into_iter().enumerate() {
        let err = insert_row(&pool, email.id, &format!("k{index}"), columns, values)
            .await
            .expect_err(name);
        assert_eq!(code(&err).as_deref(), Some("23514"), "{name}: {err}");
    }
    let err = insert_row(&pool, email.id, "pending", "", "")
        .await
        .expect_err("(email_id, input_key) is unique");
    assert_eq!(code(&err).as_deref(), Some("23505"), "{err}");

    // Deleting an email deletes its rows.
    let other = insert_email(&pool, thread, "b", "s", "2026-10-22T17:00:00Z", "b").await;
    insert_row(&pool, other.id, "x", "", "")
        .await
        .expect("another email's row");
    sqlx::query("DELETE FROM emails WHERE id = $1")
        .bind(email.id)
        .execute(&pool)
        .await
        .expect("delete the email");
    assert_eq!(
        count(&pool, "SELECT count(*) FROM email_classifications").await,
        1
    );

    // `DELETE /data` empties the table.
    let (client, _fake) = app(&pool, "hang").await;
    let response = client
        .delete("/data")
        .header(host())
        .header(ContentType::JSON)
        .body(r#"{"confirmation": "delete all data"}"#)
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM email_classifications").await,
        0
    );
}

/// `(status, run_id, claude_call_id, input_key, invalid_reasons)` of email `email_id`'s
/// only classification row.
async fn row_state(
    pool: &PgPool,
    email_id: i64,
) -> (String, Option<i64>, Option<i64>, String, Value) {
    sqlx::query_as(
        "SELECT status, run_id, claude_call_id, input_key, invalid_reasons \
         FROM email_classifications WHERE email_id = $1",
    )
    .bind(email_id)
    .fetch_one(pool)
    .await
    .expect("one classification row")
}

#[sqlx::test]
async fn worker_classifies_and_stores_the_result(pool: PgPool) {
    let email = seed_thread(&pool).await;
    let (_client, fake) = app(&pool, "classify_email").await;

    let run = start(&pool, json!({"email_id": email.id})).await;
    let output = until(&pool, run, "succeeded").await;
    let classification_id = output["classification_id"].as_i64().expect("an id");
    assert_eq!(
        output,
        json!({
            "email_id": email.id, "classification_id": classification_id,
            "status": "classified", "class": "availability_request", "confidence": 0.86,
            "company": "Soylent", "role": "Staff Engineer", "stage": "Recruiter call",
            "interviewer": "Dana Lee",
            "proposed_times": [
                {"text": "Tue Oct 27 at 2pm PT", "start_local": "2026-10-27T14:00:00",
                 "end_local": null, "time_zone": "America/Los_Angeles",
                 "start_utc": "2026-10-27T21:00:00Z", "end_utc": null},
                {"text": "Tue Nov 3 at 2pm PT", "start_local": "2026-11-03T14:00:00",
                 "end_local": null, "time_zone": null, "start_utc": null, "end_utc": null},
            ],
            "issues": [{"field": "proposed_times[1]", "reason": "unknown time zone"}],
        })
    );

    // The row holds what the step returned.
    let stored: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('email_id', email_id, 'classification_id', id, \
         'status', status, 'class', class, 'confidence', confidence, 'company', company, \
         'role', role, 'stage', stage, 'interviewer', interviewer, \
         'proposed_times', proposed_times, 'issues', issues) \
         FROM email_classifications WHERE id = $1",
    )
    .bind(classification_id)
    .fetch_one(&pool)
    .await
    .expect("read the row");
    assert_eq!(stored, output);
    let call_id: i64 =
        sqlx::query_scalar("SELECT id FROM claude_calls WHERE purpose = 'email.classify'")
            .fetch_one(&pool)
            .await
            .expect("one logged call");
    let (status, run_id, claude_call_id, key, reasons) = row_state(&pool, email.id).await;
    assert_eq!(
        (status.as_str(), run_id, claude_call_id, key, reasons),
        (
            "classified",
            Some(run),
            Some(call_id),
            email.key(),
            json!([])
        )
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM claude_calls").await, 1);
    let (input_tokens, output_tokens, cost): (i64, i64, f64) = sqlx::query_as(
        "SELECT input_tokens, output_tokens, cost_usd FROM agent_runs WHERE id = $1",
    )
    .bind(run)
    .fetch_one(&pool)
    .await
    .expect("read the run's usage");
    assert!(input_tokens > 0 && output_tokens > 0 && cost > 0.0);

    // One isolated, structured, haiku call.
    let argv = std::fs::read_to_string(fake.record.join("argv")).expect("argv");
    let args: Vec<&str> = argv.lines().collect();
    let after = |flag: &str| -> Option<&str> {
        let at = args.iter().position(|arg| *arg == flag)?;
        args.get(at + 1).copied()
    };
    assert_eq!(after("--tools"), Some(""), "{args:?}");
    assert_eq!(after("--model"), Some("haiku"), "{args:?}");
    for flag in [
        "--json-schema",
        "--strict-mcp-config",
        "--no-session-persistence",
    ] {
        assert!(args.contains(&flag), "{flag} in {args:?}");
    }
    assert!(!args.contains(&"--bare"), "{args:?}");
    let stdin = std::fs::read_to_string(fake.record.join("stdin")).expect("stdin");
    assert!(stdin.contains("Times for a recruiter call"), "{stdin}");
    assert!(stdin.contains("EARLIERMARKER"), "{stdin}");
}

#[sqlx::test]
async fn invalid_answer_fails_the_run_and_marks_the_row(pool: PgPool) {
    let email = seed_thread(&pool).await;
    let (_client, _fake) = app(&pool, "classify_email_invalid").await;

    let run = start(&pool, json!({"email_id": email.id})).await;
    until(&pool, run, "failed").await;
    let error: String = sqlx::query_scalar("SELECT error FROM agent_runs WHERE id = $1")
        .bind(run)
        .fetch_one(&pool)
        .await
        .expect("read the error");
    assert!(
        error.contains(
            "the classification is invalid: unknown class; confidence must be a number from 0 to 1"
        ),
        "{error}"
    );
    let (status, run_id, claude_call_id, _, reasons) = row_state(&pool, email.id).await;
    assert_eq!(status, "invalid");
    assert_eq!(run_id, Some(run));
    assert!(claude_call_id.is_some());
    assert_eq!(
        reasons,
        json!(["unknown class", "confidence must be a number from 0 to 1"])
    );
    let (class, confidence): (Option<String>, Option<f64>) =
        sqlx::query_as("SELECT class, confidence FROM email_classifications WHERE email_id = $1")
            .bind(email.id)
            .fetch_one(&pool)
            .await
            .expect("read the row");
    assert_eq!((class, confidence), (None, None));
}

#[sqlx::test]
async fn classified_and_superseded_runs_skip_the_model(pool: PgPool) {
    let email = seed_thread(&pool).await;
    let classified = insert_row(
        &pool,
        email.id,
        &email.key(),
        ", status, class, confidence, company",
        ", 'classified', 'offer', 0.9, 'Wayne'",
    )
    .await
    .expect("a classified row");
    let stale = insert_row(&pool, email.id, "old-key", "", "")
        .await
        .expect("a stale pending row");
    // A model call would hang, and the runs would never succeed.
    let (_client, fake) = app(&pool, "hang").await;

    // A rerun without a key finds the classified row for the current one.
    let run = start(&pool, json!({"email_id": email.id})).await;
    let output = until(&pool, run, "succeeded").await;
    assert_eq!(
        output,
        json!({"email_id": email.id, "classification_id": classified,
               "status": "classified", "class": "offer", "confidence": 0.9,
               "company": "Wayne", "role": null, "stage": null, "interviewer": null,
               "proposed_times": [], "issues": []})
    );
    // So does a run that carries the current key.
    let run = start(
        &pool,
        json!({"email_id": email.id, "input_key": email.key()}),
    )
    .await;
    assert_eq!(until(&pool, run, "succeeded").await, output);

    // A run queued for older content supersedes its row.
    let run = start(&pool, json!({"email_id": email.id, "input_key": "old-key"})).await;
    assert_eq!(
        until(&pool, run, "succeeded").await,
        json!({"email_id": email.id, "status": "superseded"})
    );
    let statuses: Vec<(i64, String, Option<i64>)> =
        sqlx::query_as("SELECT id, status, run_id FROM email_classifications ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("read the rows");
    assert_eq!(
        statuses,
        [
            (classified, "classified".into(), None),
            (stale, "superseded".into(), None)
        ]
    );

    assert_eq!(count(&pool, "SELECT count(*) FROM claude_calls").await, 0);
    assert!(!fake.record.join("argv").exists(), "the fake was never run");
}
