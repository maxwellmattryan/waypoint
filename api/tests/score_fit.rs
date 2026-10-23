//! The `score_fit` workflow end to end: `POST /postings/{id}/fit` queues a run, the worker
//! runs its `score` step against `fake-claude.sh`, and what it stores, sends to review,
//! drops and refuses; then the review decisions and the read routes.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::events::EventBus;
use waypoint_api::jobs::Wake;
use waypoint_api::runs;
use waypoint_api::workflows::{self, score_fit};

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

/// Queues a score for `posting` and returns the run's id.
async fn score(client: &Client, posting: i64) -> i64 {
    let (status, body) = send(client, Some(json!({})), &format!("/postings/{posting}/fit")).await;
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

/// In a fresh database: confirmed fact 1 (matches the posting), confirmed preference 2,
/// proposed fact 3 (matches, but never used) and confirmed fact 4 (shares no word).
async fn seed_memories(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) VALUES \
         ('fact', 'skill', 'Shipped Rust services', 'confirmed', now()), \
         ('preference', 'location', 'Remote only', 'confirmed', now()), \
         ('fact', 'skill', 'Rust compiler work', 'proposed', NULL), \
         ('fact', 'skill', 'Enjoys cooking', 'confirmed', now())",
    )
    .execute(pool)
    .await
    .expect("seed memories");
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

/// `(status, score, review_item_id)` of run `run`'s score.
async fn row(pool: &PgPool, run: i64) -> (String, i32, Option<i64>) {
    sqlx::query_as("SELECT status, score, review_item_id FROM posting_fit_scores WHERE run_id = $1")
        .bind(run)
        .fetch_one(pool)
        .await
        .expect("read the score")
}

#[sqlx::test]
async fn scores_a_posting_and_records_provenance(pool: PgPool) {
    seed_memories(&pool).await;
    let posting = seed_posting(&pool, "Acme").await;
    let (client, fake) = app(&pool, "score_fit").await;

    let run = score(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM posting_fit_scores")
        .fetch_one(&pool)
        .await
        .expect("one score");
    assert_eq!(
        output,
        json!({
            "posting_id": posting, "fit_score_id": id, "score": 82, "band": "strong",
            "confidence": 0.9, "status": "applied", "review_item_id": null,
            "dropped": {"reasons": 0, "citations": 0},
        })
    );
    let stored: (
        i64,
        i64,
        String,
        i32,
        String,
        f64,
        String,
        Value,
        Option<i64>,
        f64,
    ) = sqlx::query_as(
        "SELECT posting_id, run_id, status, score, band, confidence, summary, reasons, \
             claude_call_id, cost_usd FROM posting_fit_scores WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("read the score");
    let call: (i64, f64) = sqlx::query_as("SELECT id, cost_usd FROM claude_calls")
        .fetch_one(&pool)
        .await
        .expect("one call");
    assert_eq!(
        stored,
        (
            posting,
            run,
            "applied".into(),
            82,
            "strong".into(),
            0.9,
            "A strong Rust match".into(),
            json!([
                {"stance": "for", "text": "Has shipped Rust services", "citations": [
                    {"source": "memory", "memory_id": 1, "topic": "skill",
                     "text": "Shipped Rust services"}]},
                {"stance": "for", "text": "The team writes Rust", "citations": [
                    {"source": "posting", "memory_id": null, "topic": null, "text": "Rust"}]},
            ]),
            Some(call.0),
            call.1,
        )
    );
    assert!(call.1 > 0.0);
    assert_eq!(count(&pool, "SELECT count(*) FROM review_items").await, 0);

    // The preference and the retrieved fact; not the proposed or unrelated memories.
    let used: Vec<i64> =
        sqlx::query_scalar("SELECT memory_id FROM run_memories WHERE run_id = $1 ORDER BY id")
            .bind(run)
            .fetch_all(&pool)
            .await
            .expect("read run_memories");
    assert_eq!(used, [2, 1]);

    let argv = std::fs::read_to_string(fake.record.join("argv")).expect("argv");
    let args: Vec<&str> = argv.lines().collect();
    let after = |flag: &str| -> Option<&str> {
        let at = args.iter().position(|arg| *arg == flag)?;
        args.get(at + 1).copied()
    };
    assert_eq!(after("--tools"), Some(""), "{args:?}");
    assert_eq!(after("--model"), Some("sonnet"), "{args:?}");
    assert!(args.contains(&"--json-schema"), "{args:?}");
    let stdin = std::fs::read_to_string(fake.record.join("stdin")).expect("stdin");
    for needle in [
        "[memory 2 · preference · location] Remote only",
        "[memory 1 · fact · skill] Shipped Rust services",
        "Title: Backend Engineer",
        DESCRIPTION,
    ] {
        assert!(stdin.contains(needle), "{needle} in {stdin}");
    }
    assert!(!stdin.contains("Rust compiler work"), "{stdin}");
    assert!(!stdin.contains("Enjoys cooking"), "{stdin}");

    // The posting now shows the score.
    let listed = get(&client, "/postings").await;
    assert_eq!(listed[0]["fit_score"], 82);
    assert_eq!(listed[0]["fit_band"], "strong");
    assert_eq!(listed[0]["fit_in_review"], false);
}

#[sqlx::test]
async fn unsure_score_goes_to_review_and_accept_applies_it(pool: PgPool) {
    seed_memories(&pool).await;
    let posting = seed_posting(&pool, "Acme").await;
    seed_applied(&pool, posting, 60).await;
    let (client, _fake) = app(&pool, "score_fit_unsure").await;

    let run = score(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    assert_eq!(output["status"], "in_review", "{output}");
    let item = output["review_item_id"].as_i64().expect("a review item");
    assert_eq!(row(&pool, run).await, ("in_review".into(), 55, Some(item)));
    let items = get(&client, "/review-items").await;
    assert_eq!(items[0]["kind"], "score_fit.score");
    assert_eq!(items[0]["payload"]["company"], "Acme");
    assert_eq!(items[0]["payload"]["band"], "partial");
    // The posting's current score is unchanged.
    let fit = get(&client, &format!("/postings/{posting}/fit")).await;
    assert_eq!(fit["current"]["score"], 60);

    let accept = format!("/review-items/{item}/accept");
    let (status, body) = send(&client, Some(json!({})), &accept).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["status"], "accepted");
    assert_eq!(row(&pool, run).await.0, "applied");
    let fit = get(&client, &format!("/postings/{posting}/fit")).await;
    assert_eq!(fit["current"]["score"], 55);
    assert_eq!(fit["in_review"], Value::Null);

    let (status, _) = send(&client, Some(json!({})), &accept).await;
    assert_eq!(status, Status::Conflict);
}

#[sqlx::test]
async fn dropped_citations_send_score_to_review(pool: PgPool) {
    seed_memories(&pool).await;
    let posting = seed_posting(&pool, "Acme").await;
    let (client, _fake) = app(&pool, "score_fit_bad_citation").await;

    let run = score(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    assert_eq!(output["status"], "in_review", "{output}");
    assert_eq!(output["confidence"], 0.95);
    assert_eq!(output["dropped"], json!({"reasons": 1, "citations": 2}));
    let reasons: Value =
        sqlx::query_scalar("SELECT reasons FROM posting_fit_scores WHERE run_id = $1")
            .bind(run)
            .fetch_one(&pool)
            .await
            .expect("read the reasons");
    assert_eq!(
        reasons,
        json!([{"stance": "for", "text": "Has shipped Rust services", "citations": [
            {"source": "memory", "memory_id": 1, "topic": "skill",
             "text": "Shipped Rust services"}]}])
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM review_items WHERE status = 'pending'"
        )
        .await,
        1
    );
}

#[sqlx::test]
async fn dismissing_a_fit_review_item_keeps_the_previous_score(pool: PgPool) {
    seed_memories(&pool).await;
    let posting = seed_posting(&pool, "Acme").await;
    seed_applied(&pool, posting, 60).await;
    let (client, _fake) = app(&pool, "score_fit_unsure").await;

    let run = score(&client, posting).await;
    let (output, _) = until(&pool, run, "succeeded").await;
    let item = output["review_item_id"].as_i64().expect("a review item");
    let (status, body) = send(
        &client,
        Some(json!({"reason": "Seniority is clear"})),
        &format!("/review-items/{item}/dismiss"),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(row(&pool, run).await.0, "dismissed");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memory_signals WHERE kind = 'dismissal' \
             AND subject = 'score_fit.score'"
        )
        .await,
        1
    );
    let fit = get(&client, &format!("/postings/{posting}/fit")).await;
    assert_eq!(fit["current"]["score"], 60);
    assert_eq!(fit["in_review"], Value::Null);
    let listed = get(&client, "/postings").await;
    assert_eq!(listed[0]["fit_score"], 60);
    assert_eq!(listed[0]["fit_in_review"], false);
}

#[sqlx::test]
async fn invalid_answer_fails_the_run_and_stores_nothing(pool: PgPool) {
    seed_memories(&pool).await;
    let posting = seed_posting(&pool, "Acme").await;
    let (client, _fake) = app(&pool, "score_fit_invalid").await;

    let run = score(&client, posting).await;
    let (_, error) = until(&pool, run, "failed").await;
    let error = error.expect("an error");
    assert!(error.contains("the fit score is invalid"), "{error}");
    assert!(error.contains(score_fit::BAD_SCORE), "{error}");
    assert_eq!(
        count(&pool, "SELECT count(*) FROM posting_fit_scores").await,
        0
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM review_items").await, 0);
}

#[sqlx::test]
async fn no_memories_fails_without_a_model_call(pool: PgPool) {
    let posting = seed_posting(&pool, "Acme").await;
    // A proposed memory is not a user model.
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status) \
         VALUES ('preference', 'location', 'Remote only', 'proposed')",
    )
    .execute(&pool)
    .await
    .expect("seed a proposed memory");
    let (client, fake) = app(&pool, "score_fit").await;

    let run = score(&client, posting).await;
    let (_, error) = until(&pool, run, "failed").await;
    let error = error.expect("an error");
    assert!(error.contains(score_fit::NO_USER_MODEL), "{error}");
    assert!(!fake.record.join("argv").exists(), "the fake was never run");

    let unknown = runs::start(
        &pool,
        &EventBus::new(),
        &Wake::default(),
        &workflows::runtime(),
        score_fit::WORKFLOW,
        json!({"posting_id": 999}),
    )
    .await
    .expect("start a run");
    let (_, error) = until(&pool, unknown.id, "failed").await;
    let error = error.expect("an error");
    assert!(error.contains("no posting 999"), "{error}");
    assert_eq!(
        count(&pool, "SELECT count(*) FROM posting_fit_scores").await,
        0
    );
}

#[sqlx::test]
async fn score_route_starts_one_run_per_posting(pool: PgPool) {
    let posting = seed_posting(&pool, "Acme").await;
    // Paused, so the run waits.
    sqlx::query("UPDATE kill_switch SET paused = true")
        .execute(&pool)
        .await
        .expect("pause");
    let (client, _fake) = app(&pool, "score_fit").await;
    let path = format!("/postings/{posting}/fit");

    let (status, first) = send(&client, Some(json!({})), &path).await;
    assert_eq!(status, Status::Accepted, "{first}");
    assert_eq!(first["reused"], false);
    assert_eq!(first["run"]["workflow"], "score_fit");
    assert_eq!(first["run"]["status"], "paused");
    assert_eq!(first["run"]["input"], json!({"posting_id": posting}));
    let (status, second) = send(&client, Some(json!({})), &path).await;
    assert_eq!(status, Status::Accepted, "{second}");
    assert_eq!(second["reused"], true);
    assert_eq!(second["run"]["id"], first["run"]["id"]);
    assert_eq!(count(&pool, "SELECT count(*) FROM agent_runs").await, 1);

    let (status, _) = send(&client, Some(json!({})), "/postings/999/fit").await;
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
    assert_eq!(count(&pool, "SELECT count(*) FROM agent_runs").await, 1);
}

#[sqlx::test]
async fn score_route_refuses_cross_site_and_forms(pool: PgPool) {
    let posting = seed_posting(&pool, "Acme").await;
    let (client, _fake) = app(&pool, "score_fit").await;
    let path = format!("/postings/{posting}/fit");

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
    assert_eq!(count(&pool, "SELECT count(*) FROM agent_runs").await, 0);
}

#[sqlx::test]
async fn fit_route_and_list_show_current_and_in_review_scores(pool: PgPool) {
    seed_memories(&pool).await;
    let posting = seed_posting(&pool, "Acme").await;
    let unscored = seed_posting(&pool, "Globex").await;
    let applied = seed_applied(&pool, posting, 75).await;
    let (client, _fake) = app(&pool, "score_fit_unsure").await;

    // Never scored: nothing.
    let fit = get(&client, &format!("/postings/{unscored}/fit")).await;
    assert_eq!(
        fit,
        json!({"posting_id": unscored, "current": null, "in_review": null, "run": null})
    );
    let (status, _) = send(&client, None, "/postings/999/fit").await;
    assert_eq!(status, Status::NotFound);

    let run = score(&client, posting).await;
    until(&pool, run, "succeeded").await;
    let fit = get(&client, &format!("/postings/{posting}/fit")).await;
    assert_eq!(fit["posting_id"], posting);
    assert_eq!(fit["current"]["id"], applied);
    assert_eq!(fit["current"]["score"], 75);
    assert_eq!(fit["current"]["status"], "applied");
    let in_review = &fit["in_review"];
    assert_eq!(in_review["score"], 55);
    assert_eq!(in_review["band"], "partial");
    assert_eq!(in_review["status"], "in_review");
    assert_eq!(in_review["run_id"], run);
    assert_eq!(in_review["confidence"], 0.5);
    assert_eq!(in_review["summary"], "Unclear seniority");
    assert_eq!(in_review["reasons"][0]["citations"][0]["memory_id"], 1);
    assert!(in_review["cost_usd"].as_f64().expect("cost") > 0.0);
    assert_eq!(
        fit["run"],
        json!({"id": run, "status": "succeeded", "error": null})
    );

    let listed = get(&client, "/postings").await;
    let by_id = |id: i64| {
        listed
            .as_array()
            .expect("a list")
            .iter()
            .find(|p| p["id"] == id)
            .cloned()
            .expect("listed")
    };
    let scored = by_id(posting);
    assert_eq!(
        (
            &scored["fit_score"],
            &scored["fit_band"],
            &scored["fit_in_review"]
        ),
        (&json!(75), &json!("strong"), &json!(true))
    );
    let never = by_id(unscored);
    assert_eq!(
        (
            &never["fit_score"],
            &never["fit_band"],
            &never["fit_in_review"]
        ),
        (&Value::Null, &Value::Null, &json!(false))
    );
    let detail = get(&client, &format!("/postings/{posting}")).await;
    assert_eq!(detail["fit_score"], 75);
    assert_eq!(detail["fit_in_review"], true);

    // A deleted memory's citation keeps its snapshot but loses its id.
    sqlx::query("DELETE FROM memories WHERE id = 1")
        .execute(&pool)
        .await
        .expect("delete memory 1");
    let fit = get(&client, &format!("/postings/{posting}/fit")).await;
    assert_eq!(
        fit["in_review"]["reasons"][0]["citations"][0],
        json!({"source": "memory", "memory_id": null, "topic": "skill",
               "text": "Shipped Rust services"})
    );
}
