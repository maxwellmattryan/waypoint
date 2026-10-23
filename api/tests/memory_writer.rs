//! The memory writer end to end: `POST /memories/learn` starts or reuses a writer run, the
//! worker runs its `write` step against `fake-claude.sh`, and what it stores, reviews, drops
//! and marks processed. Decision scheduling is tested in `approvals.rs`.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::claude::ClaudeRunner;

const TIMEOUT: Duration = Duration::from_secs(20);

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The API with the production runtime and a runner playing `mode`. Keep the `Fake` alive
/// as long as the client.
async fn app(
    pool: &PgPool,
    mode: &str,
    config: waypoint_api::jobs::Config,
) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(30));
    let rocket = common::app(pool.clone(), fake.runner.clone(), config);
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// `(status, JSON body or null)` of `POST /memories/learn`.
async fn learn(client: &Client) -> (Status, Value) {
    let response = client
        .post("/memories/learn")
        .header(host())
        .dispatch()
        .await;
    let status = response.status();
    let body = response.into_json().await.unwrap_or(Value::Null);
    (status, body)
}

/// Starts (or reuses) a writer run and returns its id.
async fn learn_id(client: &Client, reused: bool) -> i64 {
    let (status, body) = learn(client).await;
    assert_eq!(status, Status::Accepted, "{body}");
    assert_eq!(body["reused"], json!(reused), "{body}");
    body["run_id"].as_i64().expect("run_id")
}

/// `n` unedited approval signals, ids 1..=n in a fresh database.
async fn seed_signals(pool: &PgPool, n: i64) {
    sqlx::query(
        "INSERT INTO memory_signals \
         (kind, run_id, pending_action_id, subject, drafted, approved, changes) \
         SELECT 'approval', 7, i, 'draft_follow_up', jsonb_build_object('message', 'm' || i), \
         jsonb_build_object('message', 'm' || i), '[]'::jsonb \
         FROM generate_series(1, $1::bigint) AS i ORDER BY i",
    )
    .bind(n)
    .execute(pool)
    .await
    .expect("seed signals");
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

async fn evidence(pool: &PgPool, memory_id: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT signal_id FROM memory_evidence WHERE memory_id = $1 ORDER BY signal_id",
    )
    .bind(memory_id)
    .fetch_all(pool)
    .await
    .expect("read evidence")
}

/// `(status, source_kind, source_ref, source_label, content, revises_memory_id)` of memory `id`.
type MemoryRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<i64>,
);

async fn memory(pool: &PgPool, id: i64) -> MemoryRow {
    sqlx::query_as(
        "SELECT status, source_kind, source_ref, source_label, content, revises_memory_id \
         FROM memories WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("read a memory")
}

/// `(processed, writer_run_id)` per signal, by id.
async fn signal_states(pool: &PgPool) -> Vec<(bool, Option<i64>)> {
    sqlx::query_as("SELECT processed_at IS NOT NULL, writer_run_id FROM memory_signals ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read signals")
}

/// The confirmed memory the `memory_writer` fixture updates: id 1 in a fresh database.
async fn seed_confirmed(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) \
         VALUES ('answer', 'notice_period', 'Two weeks', 'confirmed', now() - interval '1 day')",
    )
    .execute(pool)
    .await
    .expect("seed a memory");
}

#[sqlx::test]
async fn writer_stores_valid_proposals_and_reviews_unsure_ones(pool: PgPool) {
    seed_confirmed(&pool).await;
    seed_signals(&pool, 3).await;
    let before: (String, String, Option<String>) =
        sqlx::query_as("SELECT content, status, confirmed_at::text FROM memories WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("read memory 1");
    let (client, _fake) = app(&pool, "memory_writer", common::fast_config()).await;

    let run = learn_id(&client, false).await;
    let output = until(&pool, run, "succeeded").await;
    assert_eq!(output["signals"], json!([1, 2, 3]), "{output}");
    assert_eq!(output["updated"], json!([]), "{output}");
    assert_eq!(output["already_known"], json!(0), "{output}");
    assert_eq!(output["remaining"], json!(0), "{output}");

    // The confident create.
    let created = output["created"].as_array().expect("created");
    assert_eq!(created.len(), 1, "{output}");
    assert_eq!(
        (
            &created[0]["kind"],
            &created[0]["topic"],
            &created[0]["evidence_signal_ids"]
        ),
        (&json!("preference"), &json!("location"), &json!([1]))
    );
    let created_id = created[0]["memory_id"].as_i64().expect("memory_id");
    assert_eq!(
        memory(&pool, created_id).await,
        (
            "proposed".into(),
            "signal".into(),
            Some("1".into()),
            Some("1 decision".into()),
            "Remote only".into(),
            None
        )
    );
    assert_eq!(evidence(&pool, created_id).await, [1]);

    // The confident update of a confirmed memory becomes a revision.
    let revisions = output["revisions"].as_array().expect("revisions");
    assert_eq!(revisions.len(), 1, "{output}");
    assert_eq!(revisions[0]["revises_memory_id"], json!(1));
    assert_eq!(revisions[0]["evidence_signal_ids"], json!([2, 3]));
    let revision_id = revisions[0]["memory_id"].as_i64().expect("memory_id");
    assert_eq!(
        memory(&pool, revision_id).await,
        (
            "proposed".into(),
            "signal".into(),
            Some("2".into()),
            Some("2 decisions".into()),
            "One month's notice".into(),
            Some(1)
        )
    );
    assert_eq!(evidence(&pool, revision_id).await, [2, 3]);
    let after: (String, String, Option<String>) =
        sqlx::query_as("SELECT content, status, confirmed_at::text FROM memories WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("read memory 1");
    assert_eq!(after, before, "the confirmed memory is untouched");

    // The unsure proposal goes to review, not to memories.
    let reviewed = output["reviewed"].as_array().expect("reviewed");
    assert_eq!(reviewed.len(), 1, "{output}");
    let (kind, confidence, payload): (String, f64, Value) =
        sqlx::query_as("SELECT kind, confidence, payload FROM review_items WHERE id = $1")
            .bind(reviewed[0].as_i64().expect("id"))
            .fetch_one(&pool)
            .await
            .expect("read the review item");
    assert_eq!((kind.as_str(), confidence), ("memory_writer.proposal", 0.5));
    assert_eq!(
        payload,
        json!({"action": "create", "target_memory_id": null, "kind": "preference",
               "topic": "industry", "content": "No crypto roles", "tags": ["crypto"],
               "evidence_signal_ids": [3]})
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM memories").await, 3);

    // The four invalid proposals are listed, with fixed reasons.
    let invalid = output["invalid"].as_array().expect("invalid");
    let indexes: Vec<&Value> = invalid.iter().map(|item| &item["index"]).collect();
    assert_eq!(
        indexes,
        [&json!(3), &json!(4), &json!(5), &json!(6)],
        "{output}"
    );
    assert!(
        invalid
            .iter()
            .all(|item| item["reason"].as_str().is_some_and(|r| !r.is_empty())),
        "{output}"
    );

    // Every claimed signal is processed by this run.
    assert_eq!(signal_states(&pool).await, vec![(true, Some(run)); 3]);

    // One logged call, counted on the run.
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM claude_calls WHERE purpose = 'memory.writer'"
        )
        .await,
        1
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM claude_calls").await, 1);
    let cost: f64 = sqlx::query_scalar("SELECT cost_usd FROM agent_runs WHERE id = $1")
        .bind(run)
        .fetch_one(&pool)
        .await
        .expect("read the run's cost");
    assert!(cost > 0.0, "{cost}");
}

#[sqlx::test]
async fn writer_updates_its_own_proposal_in_place(pool: PgPool) {
    seed_signals(&pool, 3).await;
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, source_kind, source_ref, source_label) \
         VALUES ('answer', 'notice_period', 'Two weeks', 'proposed', 'signal', '1', '1 decision')",
    )
    .execute(&pool)
    .await
    .expect("seed a proposal");
    sqlx::query("INSERT INTO memory_evidence (memory_id, signal_id) VALUES (1, 1)")
        .execute(&pool)
        .await
        .expect("seed its evidence");
    let (client, _fake) = app(&pool, "memory_writer", common::fast_config()).await;

    let run = learn_id(&client, false).await;
    let output = until(&pool, run, "succeeded").await;
    assert_eq!(output["revisions"], json!([]), "{output}");
    assert_eq!(
        output["updated"],
        json!([{"memory_id": 1, "kind": "answer", "topic": "notice_period",
                "evidence_signal_ids": [2, 3]}]),
        "{output}"
    );
    assert_eq!(
        memory(&pool, 1).await,
        (
            "proposed".into(),
            "signal".into(),
            Some("1".into()),
            Some("3 decisions".into()),
            "One month's notice".into(),
            None
        )
    );
    let tags: Vec<String> = sqlx::query_scalar("SELECT tags FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .expect("read tags");
    assert_eq!(tags, ["notice"]);
    assert_eq!(evidence(&pool, 1).await, [1, 2, 3]);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memories WHERE revises_memory_id IS NOT NULL"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memories WHERE status = 'confirmed'"
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn wrong_output_stores_nothing_and_releases_signals(pool: PgPool) {
    seed_confirmed(&pool).await;
    seed_signals(&pool, 3).await;
    let (client, fake) = app(&pool, "structured_wrong", common::fast_config()).await;

    let run = learn_id(&client, false).await;
    until(&pool, run, "failed").await;
    assert_eq!(count(&pool, "SELECT count(*) FROM memories").await, 1);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM memory_evidence").await,
        0
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM review_items").await, 0);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memory_signals WHERE processed_at IS NULL"
        )
        .await,
        3
    );
    drop(client);
    drop(fake);

    // The failed run's claims lapsed: a new run takes the same signals.
    let (client, _fake) = app(&pool, "memory_writer_empty", common::fast_config()).await;
    let next = learn_id(&client, false).await;
    assert_ne!(next, run);
    let output = until(&pool, next, "succeeded").await;
    assert_eq!(output["signals"], json!([1, 2, 3]), "{output}");
    assert_eq!(signal_states(&pool).await, vec![(true, Some(next)); 3]);
}

#[sqlx::test]
async fn no_signals_makes_no_model_call(pool: PgPool) {
    let (client, fake) = app(&pool, "success", common::fast_config()).await;

    let run = learn_id(&client, false).await;
    let output = until(&pool, run, "succeeded").await;
    assert_eq!(
        output,
        json!({"signals": [], "created": [], "updated": [], "revisions": [],
               "already_known": 0, "reviewed": [], "invalid": [], "remaining": 0})
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM claude_calls").await, 0);
    assert!(!fake.record.join("argv").exists(), "the fake was never run");
}

#[sqlx::test]
async fn large_backlog_is_processed_in_batches(pool: PgPool) {
    seed_signals(&pool, 26).await;
    let (client, _fake) = app(&pool, "memory_writer_empty", common::fast_config()).await;

    let first = learn_id(&client, false).await;
    let output = until(&pool, first, "succeeded").await;
    assert_eq!(
        output["signals"],
        json!((1..=25).collect::<Vec<i64>>()),
        "{output}"
    );
    assert_eq!(output["remaining"], json!(1), "{output}");

    let (next, input): (i64, Value) = sqlx::query_as(
        "SELECT id, input FROM agent_runs WHERE workflow = 'memory_writer' AND id <> $1",
    )
    .bind(first)
    .fetch_one(&pool)
    .await
    .expect("the next batch's run");
    assert_eq!(input, json!({"trigger": "batch"}));
    let output = until(&pool, next, "succeeded").await;
    assert_eq!(output["signals"], json!([26]), "{output}");
    assert_eq!(output["remaining"], json!(0), "{output}");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memory_signals WHERE processed_at IS NULL"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'memory_writer'"
        )
        .await,
        2,
        "the last batch scheduled nothing"
    );
}

/// `run_at <= now()` for run `id`'s job.
async fn may_start_now(pool: &PgPool, id: i64) -> bool {
    sqlx::query_scalar(
        "SELECT j.run_at <= now() FROM jobs AS j JOIN agent_runs AS r ON r.job_id = j.id \
         WHERE r.id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("read the run's job")
}

async fn post_json(client: &Client, path: &str, body: Value) -> Value {
    let response = client
        .post(path.to_owned())
        .header(host())
        .header(ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;
    assert!(
        response.status().class().is_success(),
        "POST {path}: {}",
        response.status()
    );
    response.into_json().await.expect("a JSON body")
}

#[sqlx::test]
async fn learn_route_starts_or_reuses_a_run(pool: PgPool) {
    // One job at a time, and model calls that hang: a writer run that reaches its call
    // stays `running`, and the next one stays `queued` behind it.
    let config = waypoint_api::jobs::Config {
        concurrency: 1,
        ..common::fast_config()
    };
    let (client, _fake) = app(&pool, "hang", config).await;

    // A decision schedules a writer run 60 s out.
    let application = post_json(
        &client,
        "/applications",
        json!({"company": "Acme", "title": "Engineer"}),
    )
    .await["id"]
        .clone();
    let tidy = post_json(
        &client,
        &format!("/applications/{application}/tidy"),
        json!({"note": "n", "message": "Thanks", "confidence": 1.0}),
    )
    .await["id"]
        .as_i64()
        .expect("id");
    until(&pool, tidy, "awaiting_approval").await;
    let pending: i64 = sqlx::query_scalar("SELECT id FROM pending_actions WHERE run_id = $1")
        .bind(tidy)
        .fetch_one(&pool)
        .await
        .expect("the pending action");
    post_json(
        &client,
        &format!("/pending-actions/{pending}/approve"),
        json!({}),
    )
    .await;
    let scheduled: i64 = sqlx::query_scalar(
        "SELECT id FROM agent_runs WHERE workflow = 'memory_writer' AND status = 'queued'",
    )
    .fetch_one(&pool)
    .await
    .expect("the scheduled writer run");
    assert!(!may_start_now(&pool, scheduled).await);

    // The route returns that run and pulls it to now.
    assert_eq!(learn_id(&client, true).await, scheduled);
    assert!(may_start_now(&pool, scheduled).await);
    until(&pool, scheduled, "running").await;

    // A running writer isn't waiting: the route starts a new run, then reuses it.
    let started = learn_id(&client, false).await;
    assert_ne!(started, scheduled);
    let (status, input): (String, Value) =
        sqlx::query_as("SELECT status, input FROM agent_runs WHERE id = $1")
            .bind(started)
            .fetch_one(&pool)
            .await
            .expect("read the new run");
    assert_eq!(status, "queued");
    assert_eq!(input, json!({"trigger": "manual"}));
    assert!(may_start_now(&pool, started).await);
    assert_eq!(learn_id(&client, true).await, started);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'memory_writer'"
        )
        .await,
        2
    );
}

#[sqlx::test]
async fn learn_route_refuses_foreign_requests(pool: PgPool) {
    let rocket = common::app(
        pool.clone(),
        ClaudeRunner::new("claude", Duration::from_secs(1)),
        common::fast_config(),
    );
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let cases: Vec<(&str, Vec<Header<'static>>)> = vec![
        (
            "cross-site",
            vec![host(), Header::new("Sec-Fetch-Site", "cross-site")],
        ),
        (
            "form",
            vec![
                host(),
                Header::new("Content-Type", "application/x-www-form-urlencoded"),
            ],
        ),
        (
            "multipart",
            vec![
                host(),
                Header::new("Content-Type", "multipart/form-data; boundary=x"),
            ],
        ),
        ("foreign host", vec![Header::new("Host", "evil.example")]),
    ];
    for (name, headers) in cases {
        let mut request = client.post("/memories/learn").body("a=1");
        for header in headers {
            request = request.header(header);
        }
        assert_eq!(
            request.dispatch().await.status(),
            Status::Forbidden,
            "{name}"
        );
    }
    assert_eq!(count(&pool, "SELECT count(*) FROM agent_runs").await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM jobs").await, 0);
}
