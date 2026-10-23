//! Board sourcing: `POST /sourcing/boards` and the `source_board` job. Every board API is
//! wiremock, with each `capture::Config` board base set to `server.uri()`; the recorded
//! boards are under `fixtures/sourcing/`.

mod common;

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::capture;
use wiremock::matchers::{any, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TIMEOUT: Duration = Duration::from_secs(10);
const BOARD: &str = "example-labs";
const GREENHOUSE: &str = include_str!("fixtures/sourcing/greenhouse_board.json");
const LEVER_PAGES: [&str; 3] = [
    include_str!("fixtures/sourcing/lever_page_1.json"),
    include_str!("fixtures/sourcing/lever_page_2.json"),
    include_str!("fixtures/sourcing/lever_page_3.json"),
];
const ASHBY: &str = include_str!("fixtures/sourcing/ashby_board.json");

/// The board bases at wiremock, two postings per Lever page.
fn config(server: &MockServer) -> capture::Config {
    let mut config = capture::Config {
        board_page_size: 2,
        ..Default::default()
    };
    config.greenhouse.api = server.uri();
    config.lever.api = server.uri();
    config.ashby.api = server.uri();
    config
}

async fn client(pool: &PgPool, server: &MockServer) -> (Client, common::Fake) {
    let fake = common::fake("success", TIMEOUT);
    let rocket = common::app_with_capture(
        pool.clone(),
        fake.runner.clone(),
        common::fast_config(),
        config(server),
    );
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

async fn post<'c>(client: &'c Client, body: &Value) -> LocalResponse<'c> {
    client
        .post("/sourcing/boards")
        .header(host())
        .header(ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await
}

/// Queues a board through the route and returns the 202 body.
async fn enqueue(client: &Client, source: &str, board: &str) -> Value {
    let response = post(client, &json!({"source": source, "board": board})).await;
    assert_eq!(response.status(), Status::Accepted, "{source}/{board}");
    response.into_json().await.expect("queued json")
}

/// Queues a board and waits for its job to succeed or fail; returns the job.
async fn source(client: &Client, source: &str, board: &str) -> Value {
    let queued = enqueue(client, source, board).await;
    assert_eq!(queued["reused"], false, "{source}/{board}");
    let id = queued["job"]["id"].as_i64().expect("job id");
    wait_for_job(client, id).await
}

/// Polls `GET /jobs/{id}` until the job succeeds or fails, or the timeout passes.
async fn wait_for_job(client: &Client, id: i64) -> Value {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let job: Value = client
            .get(format!("/jobs/{id}"))
            .header(host())
            .dispatch()
            .await
            .into_json()
            .await
            .expect("job json");
        let finished = matches!(job["status"].as_str(), Some("succeeded" | "failed"));
        if finished || tokio::time::Instant::now() > deadline {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The result without `duration_ms`, which is checked to be a number.
fn counts(job: &Value) -> Value {
    assert_eq!(job["status"], "succeeded", "{job}");
    let mut result = job["result"].clone();
    let duration = result
        .as_object_mut()
        .expect("a result object")
        .remove("duration_ms");
    assert!(duration.is_some_and(|d| d.is_u64()), "{job}");
    result
}

fn json_body(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

async fn serve_greenhouse(server: &MockServer, board: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/boards/{board}/jobs")))
        .and(query_param("content", "true"))
        .respond_with(json_body(body))
        .mount(server)
        .await;
}

async fn serve_ashby(server: &MockServer, board: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/posting-api/job-board/{board}")))
        .respond_with(json_body(body))
        .mount(server)
        .await;
}

/// One Lever page at `skip`, expected to be asked for exactly `times`.
async fn serve_lever_page(server: &MockServer, skip: &str, response: ResponseTemplate, times: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/v0/postings/{BOARD}")))
        .and(query_param("mode", "json"))
        .and(query_param("skip", skip))
        .and(query_param("limit", "2"))
        .respond_with(response)
        .expect(times)
        .mount(server)
        .await;
}

/// The Greenhouse fixture with `edit` applied to its parsed jobs; `meta.total` follows.
fn greenhouse_with(edit: impl FnOnce(&mut Vec<Value>)) -> String {
    let mut board: Value = serde_json::from_str(GREENHOUSE).expect("fixture parses");
    let jobs = board["jobs"].as_array_mut().expect("jobs");
    edit(jobs);
    board["meta"]["total"] = json!(jobs.len());
    board.to_string()
}

/// `(external_id, title, removed_at, first_seen_at, updated_at)` for one board, by id.
type Row = (
    String,
    String,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// `(title, company, department, remote, city, region, country, description, posted_at)`.
type StoredPosting = (
    String,
    String,
    Option<String>,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<DateTime<Utc>>,
);

async fn rows(pool: &PgPool, source: &str, board: &str) -> Vec<Row> {
    sqlx::query_as(
        "SELECT external_id, title, removed_at, first_seen_at, updated_at FROM job_postings \
         WHERE source = $1 AND board = $2 ORDER BY external_id",
    )
    .bind(source)
    .bind(board)
    .fetch_all(pool)
    .await
    .expect("rows")
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

#[sqlx::test]
async fn sources_each_board(pool: PgPool) {
    let server = MockServer::start().await;
    serve_greenhouse(&server, BOARD, GREENHOUSE).await;
    for (skip, page) in ["0", "2", "4"].into_iter().zip(LEVER_PAGES) {
        serve_lever_page(&server, skip, json_body(page), 1).await;
    }
    serve_ashby(&server, BOARD, ASHBY).await;
    let (client, _fake) = client(&pool, &server).await;

    let cases = [
        ("greenhouse", 1, 3, 0, 3),
        ("lever", 3, 5, 0, 5),
        ("ashby", 1, 3, 1, 2),
    ];
    for (name, pages, fetched, skipped, inserted) in cases {
        let job = source(&client, name, BOARD).await;
        assert_eq!(
            counts(&job),
            json!({
                "source": name,
                "board": BOARD,
                "pages": pages,
                "fetched": fetched,
                "skipped": skipped,
                "inserted": inserted,
                "updated": 0,
                "removed": 0,
            }),
            "{name}"
        );
        let stored = rows(&pool, name, BOARD).await;
        assert_eq!(stored.len(), inserted, "{name}");
        assert!(stored.iter().all(|row| row.2.is_none()), "{name}: all live");
    }

    let lever: StoredPosting = sqlx::query_as(
        "SELECT title, company, department, remote, city, region, country, description, \
             posted_at FROM job_postings WHERE source = 'lever' \
             AND external_id = 'aaaaaaaa-0000-4000-8000-000000000001'",
    )
    .fetch_one(&pool)
    .await
    .expect("lever row");
    assert_eq!(
        lever,
        (
            "Staff Engineer".into(),
            "Example Labs".into(),
            Some("Engineering".into()),
            false,
            Some("San Francisco".into()),
            Some("CA".into()),
            Some("US".into()),
            "Join the platform team.\n\nWhat you'll do\n- Design APIs\n- Run Postgres\n\n\
             We sponsor visas."
                .into(),
            Some("2026-10-01T10:00:00Z".parse().unwrap()),
        )
    );
    let unlisted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_postings \
         WHERE external_id = '33333333-3333-4333-8333-333333333333'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(unlisted, 0, "the unlisted Ashby job is not stored");
}

#[sqlx::test]
async fn resourcing_updates_and_marks_removed(pool: PgPool) {
    let server = MockServer::start().await;
    let other_board = greenhouse_with(|jobs| {
        for job in jobs.iter_mut() {
            let id = job["id"].as_i64().expect("numeric id");
            job["id"] = json!(id + 1_000_000);
        }
    });
    serve_greenhouse(&server, BOARD, GREENHOUSE).await;
    serve_greenhouse(&server, "other-co", &other_board).await;
    serve_ashby(&server, BOARD, ASHBY).await;
    let (client, _fake) = client(&pool, &server).await;

    for (name, board) in [
        ("greenhouse", BOARD),
        ("greenhouse", "other-co"),
        ("ashby", BOARD),
    ] {
        let job = source(&client, name, board).await;
        assert_eq!(job["status"], "succeeded", "{name}/{board}");
    }
    let before = rows(&pool, "greenhouse", BOARD).await;
    let other_before = rows(&pool, "greenhouse", "other-co").await;
    let ashby_before = rows(&pool, "ashby", BOARD).await;
    assert_eq!(
        (before.len(), other_before.len(), ashby_before.len()),
        (3, 3, 2)
    );

    // One posting lost, another retitled.
    server.reset().await;
    let changed = greenhouse_with(|jobs| {
        jobs.retain(|job| job["id"] != 4012347);
        jobs[1]["title"] = json!("Lead Product Designer");
    });
    serve_greenhouse(&server, BOARD, &changed).await;
    let job = source(&client, "greenhouse", BOARD).await;
    let result = counts(&job);
    assert_eq!(
        (
            &result["fetched"],
            &result["inserted"],
            &result["updated"],
            &result["removed"]
        ),
        (&json!(2), &json!(0), &json!(2), &json!(1))
    );
    let after = rows(&pool, "greenhouse", BOARD).await;
    let [kept, retitled, lost] = after.as_slice() else {
        panic!("three rows: {after:?}");
    };
    assert_eq!((kept.0.as_str(), kept.2), ("4012345", None));
    assert_eq!(retitled.0, "4012346");
    assert_eq!(retitled.1, "Lead Product Designer");
    assert_eq!(retitled.2, None);
    assert_eq!(retitled.3, before[1].3, "first_seen_at is kept");
    assert!(retitled.4 > before[1].4, "updated_at moves");
    assert_eq!(lost.0, "4012347");
    assert!(lost.2.is_some(), "the lost posting is marked removed");
    assert_eq!(lost.3, before[2].3, "first_seen_at is kept");
    assert_eq!(rows(&pool, "greenhouse", "other-co").await, other_before);
    assert_eq!(rows(&pool, "ashby", BOARD).await, ashby_before);

    // The lost posting comes back.
    server.reset().await;
    serve_greenhouse(&server, BOARD, GREENHOUSE).await;
    let job = source(&client, "greenhouse", BOARD).await;
    let result = counts(&job);
    assert_eq!(
        (&result["inserted"], &result["updated"], &result["removed"]),
        (&json!(0), &json!(3), &json!(0))
    );
    let back = rows(&pool, "greenhouse", BOARD).await;
    assert!(back.iter().all(|row| row.2.is_none()), "{back:?}");
    assert_eq!(back[2].3, before[2].3, "first_seen_at survives the gap");
    assert_eq!(back[1].1, "Product Designer");

    // An empty board removes every live posting on it, and nothing else.
    server.reset().await;
    serve_greenhouse(&server, BOARD, r#"{"jobs": [], "meta": {"total": 0}}"#).await;
    let job = source(&client, "greenhouse", BOARD).await;
    let result = counts(&job);
    assert_eq!(
        (&result["fetched"], &result["removed"]),
        (&json!(0), &json!(3))
    );
    let emptied = rows(&pool, "greenhouse", BOARD).await;
    assert!(emptied.iter().all(|row| row.2.is_some()), "{emptied:?}");
    assert_eq!(rows(&pool, "greenhouse", "other-co").await, other_before);
    assert_eq!(rows(&pool, "ashby", BOARD).await, ashby_before);
}

#[sqlx::test]
async fn failed_fetch_changes_nothing(pool: PgPool) {
    let server = MockServer::start().await;
    for (skip, page) in ["0", "2", "4"].into_iter().zip(LEVER_PAGES) {
        serve_lever_page(&server, skip, json_body(page), 1).await;
    }
    let (client, _fake) = client(&pool, &server).await;
    let job = source(&client, "lever", BOARD).await;
    assert_eq!(job["status"], "succeeded");
    let before = rows(&pool, "lever", BOARD).await;
    assert_eq!(before.len(), 5);
    server.verify().await;

    // Page 2 answers 500 on every attempt; page 3 is never asked for.
    server.reset().await;
    let mut changed: Value = serde_json::from_str(LEVER_PAGES[0]).expect("fixture parses");
    changed[0]["text"] = json!("Retitled");
    serve_lever_page(&server, "0", json_body(&changed.to_string()), 3).await;
    serve_lever_page(&server, "2", ResponseTemplate::new(500), 3).await;
    serve_lever_page(&server, "4", json_body(LEVER_PAGES[2]), 0).await;
    let job = source(&client, "lever", BOARD).await;
    assert_eq!(job["status"], "failed", "{job}");
    assert_eq!(job["attempts"], 3);
    let error = job["last_error"].as_str().expect("an error");
    assert!(error.contains("page 2"), "{error}");
    assert_eq!(rows(&pool, "lever", BOARD).await, before, "nothing changed");
    server.verify().await;

    // A Greenhouse answer whose `meta.total` disagrees with its jobs.
    let partial = {
        let mut board: Value = serde_json::from_str(GREENHOUSE).expect("fixture parses");
        board["meta"]["total"] = json!(4);
        board.to_string()
    };
    Mock::given(method("GET"))
        .and(path(format!("/v1/boards/{BOARD}/jobs")))
        .respond_with(json_body(&partial))
        .expect(3)
        .mount(&server)
        .await;
    let job = source(&client, "greenhouse", BOARD).await;
    assert_eq!(job["status"], "failed", "{job}");
    assert_eq!(job["attempts"], 3);
    let error = job["last_error"].as_str().expect("an error");
    assert!(error.contains("meta.total"), "{error}");
    assert!(rows(&pool, "greenhouse", BOARD).await.is_empty());
    assert_eq!(rows(&pool, "lever", BOARD).await, before);
}

#[sqlx::test]
async fn enqueue_validates_and_dedupes(pool: PgPool) {
    let server = MockServer::start().await;
    // Slow, so the first job is still queued or running when it's asked for again.
    Mock::given(method("GET"))
        .and(path(format!("/v1/boards/{BOARD}/jobs")))
        .respond_with(json_body(GREENHOUSE).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;
    let (client, _fake) = client(&pool, &server).await;

    let first = enqueue(&client, "greenhouse", BOARD).await;
    assert_eq!(first["reused"], false);
    assert_eq!(first["job"]["kind"], "source_board");
    let again = enqueue(&client, "greenhouse", &format!("  {BOARD}  ")).await;
    assert_eq!(again["reused"], true, "the trimmed slug is the same board");
    assert_eq!(again["job"]["id"], first["job"]["id"]);

    // A queued job that may only start later is reused and pulled forward to now.
    let later: i64 = sqlx::query_scalar(
        "INSERT INTO jobs (kind, payload, max_attempts, run_at) \
         VALUES ('source_board', '{\"source\": \"lever\", \"board\": \"later\"}', 3, \
         now() + interval '1 hour') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert a delayed job");
    let reused = enqueue(&client, "lever", "later").await;
    assert_eq!(reused["reused"], true);
    assert_eq!(reused["job"]["id"], later);
    let run_at: DateTime<Utc> = reused["job"]["run_at"]
        .as_str()
        .expect("run_at")
        .parse()
        .expect("a time");
    assert!(run_at <= Utc::now(), "pulled forward: {run_at}");
    let jobs = count(&pool, "jobs").await;
    assert_eq!(jobs, 2);

    let long = "a".repeat(101);
    let source_error = json!({"field": "source", "message": "Must be greenhouse, lever or ashby"});
    let board_error = json!({
        "field": "board",
        "message": "Must be 1–100 letters, digits, hyphens or underscores",
    });
    let required = |field: &str| json!({"field": field, "message": "Required"});
    let invalid = [
        (
            json!({"source": "workday", "board": "../etc"}),
            json!([source_error, board_error]),
        ),
        (json!({}), json!([required("source"), required("board")])),
        (
            json!({"source": "lever", "board": "   "}),
            json!([required("board")]),
        ),
        (
            json!({"source": "lever", "board": long}),
            json!([board_error]),
        ),
        (
            json!({"source": "ashby", "board": "acme?x=1"}),
            json!([board_error]),
        ),
        (
            json!({"source": "Greenhouse", "board": "acme"}),
            json!([source_error]),
        ),
    ];
    for (body, errors) in invalid {
        let response = post(&client, &body).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{body}");
        let got: Value = response.into_json().await.expect("errors json");
        assert_eq!(got["errors"], errors, "{body}");
    }

    for raw in ["{", r#"{"source": 5, "board": "acme"}"#] {
        let response = client
            .post("/sourcing/boards")
            .header(host())
            .header(ContentType::JSON)
            .body(raw)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest, "{raw}");
    }
    let response = client
        .post("/sourcing/boards")
        .header(host())
        .header(ContentType::Plain)
        .body(json!({"source": "lever", "board": "acme"}).to_string())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::UnsupportedMediaType);
    assert_eq!(count(&pool, "jobs").await, jobs, "nothing else was queued");
}

#[sqlx::test]
async fn guards_refuse_cross_site_and_forms(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let (client, _fake) = client(&pool, &server).await;
    let body = json!({"source": "greenhouse", "board": "acme"}).to_string();

    let refused = [
        client
            .post("/sourcing/boards")
            .header(host())
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .header(ContentType::JSON)
            .body(body.clone()),
        client
            .post("/sourcing/boards")
            .header(host())
            .header(ContentType::Form)
            .body("source=greenhouse&board=acme"),
        client
            .post("/sourcing/boards")
            .header(host())
            .header(ContentType::Form)
            .body("_method=post&source=greenhouse&board=acme"),
        client
            .post("/sourcing/boards")
            .header(Header::new("Host", "evil.example"))
            .header(ContentType::JSON)
            .body(body.clone()),
    ];
    for request in refused {
        let response = request.dispatch().await;
        assert_eq!(response.status(), Status::Forbidden);
    }
    assert_eq!(count(&pool, "jobs").await, 0);
}
