//! The `prep_packet` workflow end to end: `POST /postings/{id}/packet` queues a run, the
//! worker runs its `prepare` step against `fake-claude.sh`, and what it stores through the
//! `save_application_packet` tool, sends to review, drops and refuses; then the review
//! decisions, the undo and the read routes.

mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::events::EventBus;
use waypoint_api::jobs::Wake;
use waypoint_api::runs;
use waypoint_api::workflows::{self, prep_packet, score_fit};

const TIMEOUT: Duration = Duration::from_secs(20);
const DESCRIPTION: &str = "Build our Rust services on PostgreSQL.";

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

/// `(status, JSON body or null)` of a request.
async fn send(client: &Client, post: Option<Value>, path: &str) -> (Status, Value) {
    let request = match &post {
        Some(body) => client
            .post(path.to_owned())
            .header(ContentType::JSON)
            .body(body.to_string()),
        None => client.get(path.to_owned()),
    };
    let response = request.header(host()).dispatch().await;
    let status = response.status();
    let body = response.into_json().await.unwrap_or(Value::Null);
    (status, body)
}

async fn get(client: &Client, path: &str) -> Value {
    let (status, body) = send(client, None, path).await;
    assert_eq!(status, Status::Ok, "GET {path}: {body}");
    body
}

/// Queues a packet for `posting` and returns the run's id.
async fn prepare(client: &Client, posting: i64) -> i64 {
    let (status, body) = send(
        client,
        Some(json!({})),
        &format!("/postings/{posting}/packet"),
    )
    .await;
    assert_eq!(status, Status::Accepted, "{body}");
    body["run"]["id"].as_i64().expect("run id")
}

/// Waits until run `id` is `status`, and returns `(output, error)`.
async fn until(pool: &PgPool, id: i64, status: &str) -> (Value, Option<String>) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let (current, output, error): (String, Option<Value>, Option<String>) =
            sqlx::query_as("SELECT status, output, error FROM agent_runs WHERE id = $1")
                .bind(id)
                .fetch_one(pool)
                .await
                .expect("read the run");
        if current == status {
            return (output.unwrap_or(Value::Null), error);
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

/// A posting whose description mentions Rust; returns its id.
async fn seed_posting(pool: &PgPool, company: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO job_postings (source, board, external_id, title, company, url, \
         location_raw, remote, description) \
         VALUES ('greenhouse', 'acme', $1, 'Backend Engineer', $1, 'https://example.com/job', \
         'Remote - EU', true, $2) RETURNING id",
    )
    .bind(company)
    .bind(DESCRIPTION)
    .fetch_one(pool)
    .await
    .expect("insert a posting")
}

/// An applied score of `score` for `posting`, from a finished run of its own.
async fn seed_applied(pool: &PgPool, posting: i64, score: i32) -> i64 {
    let run: i64 = sqlx::query_scalar(
        "INSERT INTO agent_runs (workflow, input, status) \
         VALUES ('score_fit', jsonb_build_object('posting_id', $1::bigint), 'succeeded') \
         RETURNING id",
    )
    .bind(posting)
    .fetch_one(pool)
    .await
    .expect("insert a run");
    sqlx::query_scalar(
        "INSERT INTO posting_fit_scores (posting_id, run_id, status, score, band, confidence, \
         summary, reasons) VALUES ($1, $2, 'applied', $3, 'partial', 0.9, 'Earlier', '[]') \
         RETURNING id",
    )
    .bind(posting)
    .bind(run)
    .bind(score)
    .fetch_one(pool)
    .await
    .expect("insert a score")
}

/// In a fresh database: resume 1 "Backend" and resume 2 "Frontend" with text, and resume 3
/// "Scanned" without (never shown).
async fn seed_resumes(pool: &PgPool) {
    for (name, text) in [
        (
            "Backend",
            Some("Backend engineer: Rust services and PostgreSQL"),
        ),
        ("Frontend", Some("Frontend engineer: TypeScript and Svelte")),
        ("Scanned", None),
    ] {
        sqlx::query(
            "INSERT INTO resume_versions (name, filename, size_bytes, sha256, pdf, text) \
             VALUES ($1, $1 || '.pdf', 9, md5($1), $2, $3)",
        )
        .bind(name)
        .bind(b"%PDF-test".as_slice())
        .bind(text)
        .execute(pool)
        .await
        .expect("seed a resume");
    }
}

/// In a fresh database: confirmed answer memories 1 to 4 (one per answer-bank topic),
/// confirmed fact 5 (matches the posting), confirmed preference 6 (never shown) and proposed
/// fact 7 (matches, but never used).
async fn seed_memories(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) VALUES \
         ('answer', 'work_authorization', 'EU citizen, no sponsorship needed', 'confirmed', now()), \
         ('answer', 'notice_period', 'Four weeks notice', 'confirmed', now()), \
         ('answer', 'salary_expectation', 'Around 90,000 EUR a year', 'confirmed', now()), \
         ('answer', 'why_leaving', 'Wants to own a larger platform', 'confirmed', now()), \
         ('fact', 'skill', 'Shipped Rust services', 'confirmed', now()), \
         ('preference', 'comp_floor', 'At least 80,000 EUR', 'confirmed', now()), \
         ('fact', 'skill', 'Rust compiler work', 'proposed', NULL)",
    )
    .execute(pool)
    .await
    .expect("seed memories");
}

/// A scored posting at Acme with resumes and memories; returns `(posting, fit score)`.
async fn seed_all(pool: &PgPool) -> (i64, i64) {
    seed_resumes(pool).await;
    seed_memories(pool).await;
    let posting = seed_posting(pool, "Acme").await;
    let fit = seed_applied(pool, posting, 75).await;
    (posting, fit)
}

fn memory_citation(id: i64, topic: &str, text: &str) -> Value {
    json!({"source": "memory", "memory_id": id, "topic": topic, "text": text})
}

fn question(key: &str) -> String {
    match key {
        "work_authorization" => {
            "Are you authorized to work in the role's location, and will you need visa sponsorship?"
                .into()
        }
        "notice_period" => "When could you start?".into(),
        "salary_expectation" => "What are your salary expectations?".into(),
        "why_leaving" => "Why are you looking to leave your current role?".into(),
        _ => "Why do you want to work at Acme?".into(),
    }
}

/// The five answers of the `prep_packet` fixture, as stored.
fn stored_answers() -> Value {
    json!([
        {"key": "work_authorization", "question": question("work_authorization"),
         "answer": "Yes. I am an EU citizen and need no visa sponsorship.",
         "citations": [memory_citation(1, "work_authorization", "EU citizen, no sponsorship needed")]},
        {"key": "notice_period", "question": question("notice_period"),
         "answer": "I can start four weeks after an offer.",
         "citations": [memory_citation(2, "notice_period", "Four weeks notice")]},
        {"key": "salary_expectation", "question": question("salary_expectation"),
         "answer": "Around 90,000 EUR a year.",
         "citations": [memory_citation(3, "salary_expectation", "Around 90,000 EUR a year")]},
        {"key": "why_leaving", "question": question("why_leaving"),
         "answer": "I want to own a larger platform.",
         "citations": [memory_citation(4, "why_leaving", "Wants to own a larger platform")]},
        {"key": "why_company", "question": question("why_company"),
         "answer": "Your team builds Rust services, which is what I do best.",
         "citations": [memory_citation(5, "skill", "Shipped Rust services")]},
    ])
}

fn stored_reasons() -> Value {
    json!([{"text": "The backend resume matches a Rust services role", "citations": [
        {"source": "posting", "memory_id": null, "topic": null, "text": "Rust"},
        memory_citation(5, "skill", "Shipped Rust services"),
    ]}])
}

/// `(status, review_item_id)` of run `run`'s packet.
async fn row(pool: &PgPool, run: i64) -> (String, Option<i64>) {
    sqlx::query_as("SELECT status, review_item_id FROM application_packets WHERE run_id = $1")
        .bind(run)
        .fetch_one(pool)
        .await
        .expect("read the packet")
}

#[sqlx::test]
async fn prepares_a_packet_and_records_provenance(pool: PgPool) {
    let (posting, fit) = seed_all(&pool).await;
    let (client, fake) = app(&pool, "prep_packet").await;

    let run = prepare(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM application_packets")
        .fetch_one(&pool)
        .await
        .expect("one packet");
    assert_eq!(
        output,
        json!({
            "posting_id": posting, "packet_id": id, "resume_version_id": 1,
            "confidence": 0.9, "status": "ready", "review_item_id": null, "missing": [],
            "dropped": {"resume_reasons": 0, "citations": 0, "answers": 0},
        })
    );
    type Choice = (i64, i64, String, Option<i64>, String, Value, Value);
    let choice: Choice = sqlx::query_as(
        "SELECT posting_id, run_id, status, resume_version_id, resume_name, resume_reasons, \
         answers FROM application_packets WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("read the packet");
    type Snapshot = (
        Option<i64>,
        Option<i32>,
        Option<String>,
        f64,
        Option<i64>,
        Option<i64>,
        f64,
    );
    let snapshot: Snapshot = sqlx::query_as(
        "SELECT fit_score_id, fit_score, fit_band, confidence, review_item_id, \
         claude_call_id, cost_usd FROM application_packets WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("read the packet");
    let call: (i64, f64) = sqlx::query_as("SELECT id, cost_usd FROM claude_calls")
        .fetch_one(&pool)
        .await
        .expect("one call");
    assert_eq!(
        choice,
        (
            posting,
            run,
            "ready".into(),
            Some(1),
            "Backend".into(),
            stored_reasons(),
            stored_answers(),
        )
    );
    assert_eq!(
        snapshot,
        (
            Some(fit),
            Some(75),
            Some("partial".into()),
            0.9,
            None,
            Some(call.0),
            call.1,
        )
    );
    assert!(call.1 > 0.0);
    assert_eq!(count(&pool, "SELECT count(*) FROM review_items").await, 0);
    let tool_calls: Vec<(String, String, String)> =
        sqlx::query_as("SELECT tool, permission, status FROM tool_calls")
            .fetch_all(&pool)
            .await
            .expect("read tool calls");
    assert_eq!(
        tool_calls,
        [(
            "save_application_packet".into(),
            "write".into(),
            "succeeded".into()
        )]
    );
    let actions: Vec<(String, Value)> = sqlx::query_as("SELECT tool, undo_data FROM agent_actions")
        .fetch_all(&pool)
        .await
        .expect("read actions");
    assert_eq!(
        actions,
        [("save_application_packet".into(), json!({"packet_id": id}))]
    );

    // The answer bank and the retrieved fact; not the preference or the proposed memory.
    let used: BTreeSet<i64> =
        sqlx::query_scalar::<_, i64>("SELECT memory_id FROM run_memories WHERE run_id = $1")
            .bind(run)
            .fetch_all(&pool)
            .await
            .expect("read run_memories")
            .into_iter()
            .collect();
    assert_eq!(used, BTreeSet::from([1, 2, 3, 4, 5]));

    let argv = std::fs::read_to_string(fake.record.join("argv")).expect("argv");
    let args: Vec<&str> = argv.lines().collect();
    let after = |flag: &str| -> Option<&str> {
        let at = args.iter().position(|arg| *arg == flag)?;
        args.get(at + 1).copied()
    };
    assert_eq!(after("--tools"), Some(""), "{args:?}");
    assert_eq!(after("--model"), Some("sonnet"), "{args:?}");
    assert!(args.contains(&"--json-schema"), "{args:?}");
    assert!(!args.contains(&"--mcp-config"), "{args:?}");
    let stdin = std::fs::read_to_string(fake.record.join("stdin")).expect("stdin");
    for needle in [
        "[memory 1 · answer · work_authorization] EU citizen, no sponsorship needed",
        "[memory 2 · answer · notice_period] Four weeks notice",
        "[memory 3 · answer · salary_expectation] Around 90,000 EUR a year",
        "[memory 4 · answer · why_leaving] Wants to own a larger platform",
        "[memory 5 · fact · skill] Shipped Rust services",
        "[resume 1 · Backend]",
        "[resume 2 · Frontend]",
        "Fit: 75 (partial) — Earlier",
        "Title: Backend Engineer",
        DESCRIPTION,
    ] {
        assert!(stdin.contains(needle), "{needle} in {stdin}");
    }
    for absent in ["At least 80,000", "Rust compiler work", "[resume 3"] {
        assert!(!stdin.contains(absent), "{absent} in {stdin}");
    }
}

#[sqlx::test]
async fn missing_answer_goes_to_review_and_accept_makes_it_ready(pool: PgPool) {
    let (posting, _) = seed_all(&pool).await;
    let (client, _fake) = app(&pool, "prep_packet_missing").await;

    let run = prepare(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    assert_eq!(output["status"], "in_review", "{output}");
    assert_eq!(output["confidence"], 0.9);
    assert_eq!(output["missing"], json!(["salary_expectation"]));
    assert_eq!(
        output["dropped"],
        json!({"resume_reasons": 0, "citations": 0, "answers": 0})
    );
    let item = output["review_item_id"].as_i64().expect("a review item");
    assert_eq!(row(&pool, run).await, ("in_review".into(), Some(item)));
    let answers: Value =
        sqlx::query_scalar("SELECT answers FROM application_packets WHERE run_id = $1")
            .bind(run)
            .fetch_one(&pool)
            .await
            .expect("read the answers");
    assert_eq!(
        answers[2],
        json!({"key": "salary_expectation", "question": question("salary_expectation"),
               "answer": null, "citations": []})
    );
    let items = get(&client, "/review-items").await;
    assert_eq!(items.as_array().map(Vec::len), Some(1), "{items}");
    assert_eq!(items[0]["kind"], "prep_packet.packet");
    assert_eq!(items[0]["status"], "pending");
    let payload = &items[0]["payload"];
    assert_eq!(payload["posting_id"], posting);
    assert_eq!(payload["company"], "Acme");
    assert_eq!(payload["title"], "Backend Engineer");
    assert_eq!(payload["resume"]["id"], 1);
    assert_eq!(payload["resume"]["name"], "Backend");
    assert_eq!(payload["missing"], json!(["salary_expectation"]));
    assert_eq!(payload["answers"].as_array().map(Vec::len), Some(5));

    let accept = format!("/review-items/{item}/accept");
    let (status, body) = send(&client, Some(json!({})), &accept).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["status"], "accepted");
    assert_eq!(row(&pool, run).await.0, "ready");

    let (status, _) = send(&client, Some(json!({})), &accept).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(row(&pool, run).await.0, "ready");
}

#[sqlx::test]
async fn ungrounded_answers_are_dropped_and_force_review(pool: PgPool) {
    let (posting, _) = seed_all(&pool).await;
    let (client, _fake) = app(&pool, "prep_packet_bad_citation").await;

    let run = prepare(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    assert_eq!(output["status"], "in_review", "{output}");
    assert_eq!(output["confidence"], 0.95);
    // The unshown-memory reason and its citation, the excerpt not in the posting, the
    // repeated key and the why_leaving answer that cites only a fact.
    assert_eq!(
        output["dropped"],
        json!({"resume_reasons": 1, "citations": 2, "answers": 2})
    );
    assert_eq!(output["missing"], json!(["why_leaving"]));
    let (reasons, answers): (Value, Value) =
        sqlx::query_as("SELECT resume_reasons, answers FROM application_packets WHERE run_id = $1")
            .bind(run)
            .fetch_one(&pool)
            .await
            .expect("read the packet");
    assert_eq!(
        reasons,
        json!([{"text": "The backend resume matches a Rust services role", "citations": [
            memory_citation(5, "skill", "Shipped Rust services")]}])
    );
    assert_eq!(
        answers[1]["answer"],
        "I can start four weeks after an offer."
    );
    assert_eq!(
        answers[3],
        json!({"key": "why_leaving", "question": question("why_leaving"),
               "answer": null, "citations": []})
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM review_items WHERE status = 'pending' \
             AND kind = 'prep_packet.packet'"
        )
        .await,
        1
    );
}

#[sqlx::test]
async fn dismissing_a_packet_review_item_marks_it_dismissed(pool: PgPool) {
    let (posting, _) = seed_all(&pool).await;
    let (client, _fake) = app(&pool, "prep_packet_missing").await;

    let run = prepare(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    let item = output["review_item_id"].as_i64().expect("a review item");
    let (status, body) = send(
        &client,
        Some(json!({"reason": "The salary is in the bank"})),
        &format!("/review-items/{item}/dismiss"),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(row(&pool, run).await.0, "dismissed");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memory_signals WHERE kind = 'dismissal' \
             AND subject = 'prep_packet.packet'"
        )
        .await,
        1
    );
    let listed = get(&client, &format!("/postings/{posting}/packets")).await;
    assert_eq!(listed["packets"][0]["status"], "dismissed");
}

#[sqlx::test]
async fn invalid_answer_fails_the_run_and_stores_nothing(pool: PgPool) {
    let (posting, _) = seed_all(&pool).await;
    let (client, _fake) = app(&pool, "prep_packet_invalid").await;

    let run = prepare(&client, posting).await;
    let (_, error) = until(&pool, run, "failed").await;
    let error = error.expect("an error");
    assert!(error.contains("the packet is invalid"), "{error}");
    assert!(error.contains(prep_packet::BAD_RESUME), "{error}");
    for table in [
        "application_packets",
        "review_items",
        "tool_calls",
        "agent_actions",
    ] {
        assert_eq!(
            count(&pool, &format!("SELECT count(*) FROM {table}")).await,
            0,
            "{table}"
        );
    }
}

#[sqlx::test]
async fn fails_without_a_model_call_when_nothing_to_draft_from(pool: PgPool) {
    let posting = seed_posting(&pool, "Acme").await;
    seed_resumes(&pool).await;
    // A preference and a proposed fact: neither is something to draft answers from.
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) VALUES \
         ('preference', 'comp_floor', 'At least 80,000 EUR', 'confirmed', now()), \
         ('fact', 'skill', 'Rust compiler work', 'proposed', NULL)",
    )
    .execute(&pool)
    .await
    .expect("seed memories");
    let (_client, fake) = app(&pool, "prep_packet").await;
    let start = |input: Value| {
        let pool = pool.clone();
        async move {
            runs::start(
                &pool,
                &EventBus::new(),
                &Wake::default(),
                &workflows::runtime(),
                prep_packet::WORKFLOW,
                input,
            )
            .await
            .expect("start a run")
            .id
        }
    };

    let run = start(json!({"posting_id": posting})).await;
    let (_, error) = until(&pool, run, "failed").await;
    let error = error.expect("an error");
    assert!(error.contains(score_fit::NO_USER_MODEL), "{error}");

    // With memories but no resume that has text.
    seed_memories(&pool).await;
    sqlx::query("UPDATE resume_versions SET text = '  '")
        .execute(&pool)
        .await
        .expect("blank the resumes");
    let run = start(json!({"posting_id": posting})).await;
    let (_, error) = until(&pool, run, "failed").await;
    let error = error.expect("an error");
    assert!(error.contains(prep_packet::NO_RESUME), "{error}");

    let run = start(json!({"posting_id": 999})).await;
    let (_, error) = until(&pool, run, "failed").await;
    let error = error.expect("an error");
    assert!(error.contains("no posting 999"), "{error}");

    assert!(!fake.record.join("argv").exists(), "the fake was never run");
    assert_eq!(
        count(&pool, "SELECT count(*) FROM application_packets").await,
        0
    );
}

#[sqlx::test]
async fn undoing_the_save_deletes_the_packet(pool: PgPool) {
    let (posting, _) = seed_all(&pool).await;
    let (client, _fake) = app(&pool, "prep_packet_missing").await;

    let run = prepare(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    let item = output["review_item_id"].as_i64().expect("a review item");

    // A rerun of the finished step returns the stored output: no second call or save.
    sqlx::query("UPDATE run_steps SET status = 'failed' WHERE run_id = $1")
        .bind(run)
        .execute(&pool)
        .await
        .expect("reset the step");
    sqlx::query("UPDATE agent_runs SET status = 'failed' WHERE id = $1")
        .bind(run)
        .execute(&pool)
        .await
        .expect("fail the run");
    runs::resume(&pool, &EventBus::new(), &Wake::default(), run)
        .await
        .expect("resume")
        .expect("a resumed run");
    let (again, _) = until(&pool, run, "succeeded").await;
    assert_eq!(again, output);
    assert_eq!(count(&pool, "SELECT count(*) FROM claude_calls").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM tool_calls").await, 1);

    let action: i64 = sqlx::query_scalar("SELECT id FROM agent_actions")
        .fetch_one(&pool)
        .await
        .expect("one action");
    let undo = format!("/actions/{action}/undo");
    let (status, body) = send(&client, Some(json!({})), &undo).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(
        count(&pool, "SELECT count(*) FROM application_packets").await,
        0
    );
    let item_status: String = sqlx::query_scalar("SELECT status FROM review_items WHERE id = $1")
        .bind(item)
        .fetch_one(&pool)
        .await
        .expect("read the item");
    assert_eq!(item_status, "dismissed");
    // The packet is gone now.
    let (status, _) = send(&client, Some(json!({})), &undo).await;
    assert_eq!(status, Status::Conflict);

    // A packet that was decided (here dismissed) can't be undone, and stays.
    let other = seed_posting(&pool, "Globex").await;
    seed_applied(&pool, other, 60).await;
    let run = prepare(&client, other).await;
    until(&pool, run, "succeeded").await;
    sqlx::query("UPDATE application_packets SET status = 'dismissed' WHERE run_id = $1")
        .bind(run)
        .execute(&pool)
        .await
        .expect("decide the packet");
    let action: i64 = sqlx::query_scalar("SELECT max(id) FROM agent_actions")
        .fetch_one(&pool)
        .await
        .expect("the second action");
    let (status, _) = send(&client, Some(json!({})), &format!("/actions/{action}/undo")).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(row(&pool, run).await.0, "dismissed");
}

#[sqlx::test]
async fn packet_route_queues_one_run_per_posting(pool: PgPool) {
    let posting = seed_posting(&pool, "Acme").await;
    seed_applied(&pool, posting, 75).await;
    let unscored = seed_posting(&pool, "Globex").await;
    // Paused, so the run waits.
    sqlx::query("UPDATE kill_switch SET paused = true")
        .execute(&pool)
        .await
        .expect("pause");
    let (client, _fake) = app(&pool, "prep_packet").await;
    let path = format!("/postings/{posting}/packet");
    let runs = "SELECT count(*) FROM agent_runs WHERE workflow = 'prep_packet'";

    let (status, first) = send(&client, Some(json!({})), &path).await;
    assert_eq!(status, Status::Accepted, "{first}");
    assert_eq!(first["reused"], false);
    assert_eq!(first["run"]["workflow"], "prep_packet");
    assert_eq!(first["run"]["status"], "paused");
    assert_eq!(first["run"]["input"], json!({"posting_id": posting}));
    let (status, second) = send(&client, Some(json!({})), &path).await;
    assert_eq!(status, Status::Accepted, "{second}");
    assert_eq!(second["reused"], true);
    assert_eq!(second["run"]["id"], first["run"]["id"]);
    assert_eq!(count(&pool, runs).await, 1);

    let (status, _) = send(
        &client,
        Some(json!({})),
        &format!("/postings/{unscored}/packet"),
    )
    .await;
    assert_eq!(status, Status::Conflict);
    let (status, _) = send(&client, Some(json!({})), "/postings/999/packet").await;
    assert_eq!(status, Status::NotFound);
    let response = client
        .post(path.clone())
        .header(host())
        .header(ContentType::Plain)
        .body("{}")
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::UnsupportedMediaType);
    let (status, _) = send(&client, Some(json!({"force": true})), &path).await;
    assert_eq!(status, Status::BadRequest);
    assert_eq!(count(&pool, runs).await, 1);
}

#[sqlx::test]
async fn packet_route_refuses_cross_site_and_forms(pool: PgPool) {
    let posting = seed_posting(&pool, "Acme").await;
    seed_applied(&pool, posting, 75).await;
    let (client, _fake) = app(&pool, "prep_packet").await;
    let path = format!("/postings/{posting}/packet");

    let response = client
        .post(path.clone())
        .header(host())
        .header(ContentType::JSON)
        .header(Header::new("Sec-Fetch-Site", "cross-site"))
        .body("{}")
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Forbidden, "cross-site");
    for (form, body) in [
        (ContentType::Form, "a=b"),
        (ContentType::Form, "_method=post&a=b"),
        (ContentType::FormData, "a=b"),
    ] {
        let response = client
            .post(path.clone())
            .header(host())
            .header(form.clone())
            .body(body)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden, "{form} {body}");
    }
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'prep_packet'"
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn packet_routes_list_and_show_packets(pool: PgPool) {
    let (posting, fit) = seed_all(&pool).await;
    let unscored = seed_posting(&pool, "Globex").await;
    let (client, _fake) = app(&pool, "prep_packet").await;

    // Never scored or prepared: nothing.
    let listed = get(&client, &format!("/postings/{unscored}/packets")).await;
    assert_eq!(
        listed,
        json!({"posting_id": unscored, "scored": false, "packets": [], "run": null})
    );
    for path in ["/postings/999/packets", "/packets/999"] {
        let (status, _) = send(&client, None, path).await;
        assert_eq!(status, Status::NotFound, "{path}");
    }

    let run = prepare(&client, posting).await;
    until(&pool, run, "succeeded").await;
    let listed = get(&client, &format!("/postings/{posting}/packets")).await;
    assert_eq!(listed["posting_id"], posting);
    assert_eq!(listed["scored"], true);
    assert_eq!(
        listed["run"],
        json!({"id": run, "status": "succeeded", "error": null})
    );
    let packets = listed["packets"].as_array().expect("a list");
    assert_eq!(packets.len(), 1, "{listed}");
    let packet = &packets[0];
    let id = packet["id"].as_i64().expect("an id");
    let mut expected = json!({
        "id": id, "posting_id": posting, "company": "Acme", "title": "Backend Engineer",
        "run_id": run, "status": "ready", "resume_version_id": 1, "resume_name": "Backend",
        "resume_reasons": stored_reasons(), "answers": stored_answers(),
        "fit_score_id": fit, "fit_score": 75, "fit_band": "partial", "confidence": 0.9,
        "review_item_id": null,
    });
    for key in ["cost_usd", "created_at", "updated_at"] {
        assert!(!packet[key].is_null(), "{key}");
        expected[key] = packet[key].clone();
    }
    assert_eq!(packet, &expected);
    assert_eq!(get(&client, &format!("/packets/{id}")).await, expected);

    // A deleted memory's citations keep their snapshots but lose their ids, and a deleted
    // resume keeps its name.
    sqlx::query("DELETE FROM memories WHERE id = 5")
        .execute(&pool)
        .await
        .expect("delete memory 5");
    sqlx::query("DELETE FROM resume_versions WHERE id = 1")
        .execute(&pool)
        .await
        .expect("delete resume 1");
    let shown = get(&client, &format!("/packets/{id}")).await;
    assert_eq!(shown["resume_version_id"], Value::Null);
    assert_eq!(shown["resume_name"], "Backend");
    let gone = json!({"source": "memory", "memory_id": null, "topic": "skill",
                      "text": "Shipped Rust services"});
    assert_eq!(shown["resume_reasons"][0]["citations"][1], gone);
    assert_eq!(shown["answers"][4]["citations"][0], gone);
    assert_eq!(shown["answers"][0]["citations"][0]["memory_id"], 1);
    let listed = get(&client, &format!("/postings/{posting}/packets")).await;
    assert_eq!(listed["packets"][0]["answers"][4]["citations"][0], gone);
}
