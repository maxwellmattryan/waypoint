//! `POST /captures`: the JSON-LD, job-board and Claude tiers, fetch failures, URL
//! validation, the request guards and the redirect guard. Pages are served by wiremock (or
//! a raw socket where wiremock can't shape the response); `claude` is `fake-claude.sh`.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use waypoint_api::capture;
use wiremock::matchers::{any, headers, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TIMEOUT: Duration = Duration::from_secs(10);
const JSONLD_BASIC: &str = include_str!("fixtures/capture/jsonld_basic.html");
const JSONLD_NONE: &str = include_str!("fixtures/capture/jsonld_none.html");
const GREENHOUSE_JOB: &str = include_str!("fixtures/capture/greenhouse_job.json");
const REDIRECT_REFUSED: &str =
    "The link redirects to a private network address, so Waypoint didn't follow it.";

async fn client(pool: PgPool, mode: &str, config: capture::Config) -> (Client, common::Fake) {
    let fake = common::fake(mode, TIMEOUT);
    let rocket = common::app_with_capture(pool, fake.runner.clone(), common::fast_config(), config);
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

async fn post<'c>(client: &'c Client, body: &Value) -> LocalResponse<'c> {
    client
        .post("/captures")
        .header(Header::new("Host", common::HOST))
        .header(ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await
}

async fn capture_url<'c>(client: &'c Client, url: &str) -> LocalResponse<'c> {
    post(client, &json!({ "url": url })).await
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

fn html(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/html; charset=utf-8")
}

fn json_body(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

async fn serve(server: &MockServer, at: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(response)
        .mount(server)
        .await;
}

/// Polls `GET /jobs/{id}` until `done` holds or the timeout passes.
async fn wait_for_job(client: &Client, id: i64, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let job: Value = client
            .get(format!("/jobs/{id}"))
            .header(Header::new("Host", common::HOST))
            .dispatch()
            .await
            .into_json()
            .await
            .expect("job json");
        if done(&job) || tokio::time::Instant::now() > deadline {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A one-shot raw HTTP server that writes `response` to every connection, for shapes
/// wiremock can't produce (a chunked body without `Content-Length`).
async fn raw_server(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let response = response.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{addr}/stream")
}

#[sqlx::test]
async fn json_ld_page_is_captured_without_a_job(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jobs/senior-engineer"))
        .and(headers(
            "accept",
            vec!["text/html", "application/xhtml+xml"],
        ))
        .respond_with(html(JSONLD_BASIC))
        .expect(1)
        .mount(&server)
        .await;
    let (client, _fake) = client(pool.clone(), "capture", capture::Config::default()).await;

    let url = format!("{}/jobs/senior-engineer", server.uri());
    let response = capture_url(&client, &format!("  {url} ")).await;

    assert_eq!(response.status(), Status::Ok);
    let body: Value = response.into_json().await.expect("json");
    assert_eq!(
        body,
        json!({
            "method": "json_ld",
            "fields": {
                "company": "Acme Corp",
                "title": "Senior Engineer",
                "url": url,
                "location": "San Francisco, CA, US",
                "comp": "USD 180,000–210,000 per year",
            },
        })
    );
    assert_eq!(count(&pool, "jobs").await, 0);
    assert_eq!(count(&pool, "applications").await, 0);
}

/// Capture settings whose Greenhouse hosts and API are the given wiremock.
fn greenhouse_at(server: &MockServer) -> capture::Config {
    let mut config = capture::Config::default();
    config.greenhouse.hosts = vec![server.address().to_string()];
    config.greenhouse.api = server.uri();
    config
}

#[sqlx::test]
async fn ats_tier_runs_after_json_ld(pool: PgPool) {
    let server = MockServer::start().await;
    serve(
        &server,
        "/acme/jobs/123",
        html("<p>A page without JSON-LD.</p>"),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/v1/boards/acme/jobs/123"))
        .and(query_param("pay_transparency", "true"))
        .respond_with(json_body(GREENHOUSE_JOB))
        .expect(1)
        .mount(&server)
        .await;
    let (client, _fake) = client(pool.clone(), "capture", greenhouse_at(&server)).await;

    let url = format!("{}/acme/jobs/123", server.uri());
    let response = capture_url(&client, &url).await;

    assert_eq!(response.status(), Status::Ok);
    let body: Value = response.into_json().await.expect("json");
    assert_eq!(
        body,
        json!({
            "method": "greenhouse",
            "fields": {
                "company": "Example Labs",
                "title": "Staff Software Engineer, Platform",
                "url": url,
                "location": "New York, NY",
                "comp": "USD 180,000–210,000",
            },
        })
    );
    assert_eq!(count(&pool, "jobs").await, 0);
}

#[sqlx::test]
async fn ats_tier_rescues_a_failed_page(pool: PgPool) {
    let server = MockServer::start().await;
    serve(&server, "/acme/jobs/123", ResponseTemplate::new(500)).await;
    serve(
        &server,
        "/v1/boards/acme/jobs/123",
        json_body(GREENHOUSE_JOB),
    )
    .await;
    serve(&server, "/acme/jobs/456", ResponseTemplate::new(500)).await;
    serve(
        &server,
        "/v1/boards/acme/jobs/456",
        ResponseTemplate::new(404),
    )
    .await;
    let (client, _fake) = client(pool.clone(), "capture", greenhouse_at(&server)).await;

    let rescued = capture_url(&client, &format!("{}/acme/jobs/123", server.uri())).await;
    assert_eq!(rescued.status(), Status::Ok);
    let body: Value = rescued.into_json().await.expect("json");
    assert_eq!(body["method"], "greenhouse");
    assert_eq!(body["fields"]["title"], "Staff Software Engineer, Platform");

    let both_failed = capture_url(&client, &format!("{}/acme/jobs/456", server.uri())).await;
    assert_eq!(both_failed.status(), Status::BadGateway);
    let body: Value = both_failed.into_json().await.expect("json");
    assert_eq!(
        body,
        json!({"reason": "http_status", "message": "The site answered with HTTP 500."}),
        "the page's failure, not the API's"
    );
    assert_eq!(count(&pool, "jobs").await, 0);
}

#[sqlx::test]
async fn fallback_extracts_with_claude(pool: PgPool) {
    let server = MockServer::start().await;
    serve(&server, "/careers/platform", html(JSONLD_NONE)).await;
    let (client, fake) = client(pool.clone(), "capture", capture::Config::default()).await;

    let url = format!("{}/careers/platform", server.uri());
    let response = capture_url(&client, &url).await;
    assert_eq!(response.status(), Status::Accepted);
    let body: Value = response.into_json().await.expect("json");
    let id = body["job_id"].as_i64().expect("job_id");
    assert_eq!(body, json!({"job_id": id}));

    let job = wait_for_job(&client, id, |job| job["status"] == "succeeded").await;
    assert_eq!(job["status"], "succeeded", "{job}");
    assert_eq!(job["kind"], "capture_extract");
    assert_eq!(
        job["result"],
        json!({
            "method": "llm",
            "fields": {
                "company": "Hooli",
                "title": "Platform Engineer",
                "url": url,
                "location": null,
                "comp": "x".repeat(200),
            },
        })
    );
    let echoed = job.to_string();
    assert!(
        !echoed.contains("Pay: $150k"),
        "the payload is not echoed: {echoed}"
    );
    assert!(job.get("payload").is_none());

    let argv = std::fs::read_to_string(fake.record.join("argv")).expect("argv");
    let args: Vec<&str> = argv.lines().collect();
    let base = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
    ];
    assert_eq!(&args[..base.len()], base);
    let schema_at = args
        .iter()
        .position(|arg| *arg == "--json-schema")
        .expect("--json-schema");
    let schema: Value = serde_json::from_str(args[schema_at + 1]).expect("schema json");
    assert_eq!(schema, capture::llm::schema());
    assert!(!args.contains(&"--bare"));

    let stdin = std::fs::read_to_string(fake.record.join("stdin")).expect("stdin");
    assert!(stdin.contains("Pay: $150k to $190k"), "{stdin}");
    assert!(
        stdin.contains("Hooli careers: Platform Engineer"),
        "{stdin}"
    );
    assert!(
        !stdin.contains("not text"),
        "script text is dropped: {stdin}"
    );
    assert_eq!(count(&pool, "applications").await, 0);

    let (purpose,): (String,) = sqlx::query_as("SELECT purpose FROM claude_calls")
        .fetch_one(&pool)
        .await
        .expect("one call");
    assert_eq!(purpose, "capture.extract");
}

#[sqlx::test]
async fn fallback_failures_follow_the_queue_rules(pool: PgPool) {
    let server = MockServer::start().await;
    serve(&server, "/careers/platform", html(JSONLD_NONE)).await;
    let url = format!("{}/careers/platform", server.uri());

    let (wrong, _fake) = client(pool.clone(), "structured_wrong", capture::Config::default()).await;
    let body: Value = capture_url(&wrong, &url)
        .await
        .into_json()
        .await
        .expect("json");
    let id = body["job_id"].as_i64().expect("job_id");
    let job = wait_for_job(&wrong, id, |job| job["status"] == "failed").await;
    assert_eq!(job["status"], "failed", "{job}");
    assert_eq!(job["attempts"], 3, "retried up to max_attempts");
    assert!(
        job["last_error"]
            .as_str()
            .is_some_and(|e| e.contains("does not match the schema")),
        "{job}"
    );
    assert_eq!(job["result"], Value::Null);
    drop(wrong);

    let (client, _fake) = client(pool.clone(), "usage_limit", capture::Config::default()).await;
    let body: Value = capture_url(&client, &url)
        .await
        .into_json()
        .await
        .expect("json");
    let id = body["job_id"].as_i64().expect("job_id");
    let job = wait_for_job(&client, id, |job| !job["last_error"].is_null()).await;
    assert_eq!(job["status"], "queued", "deferred: {job}");
    assert_eq!(job["attempts"], 0, "the attempt is given back");
    assert!(
        job["last_error"]
            .as_str()
            .is_some_and(|e| e.contains("usage limit")),
        "{job}"
    );
}

#[sqlx::test]
async fn fetch_failures_are_reported(pool: PgPool) {
    let server = MockServer::start().await;
    let big = "a".repeat(2048);
    serve(&server, "/big", html(&big)).await;
    serve(
        &server,
        "/slow",
        html("<p>late</p>").set_delay(Duration::from_secs(2)),
    )
    .await;
    serve(&server, "/missing", ResponseTemplate::new(404)).await;
    serve(
        &server,
        "/file.pdf",
        ResponseTemplate::new(200).set_body_raw(b"%PDF-1.7".to_vec(), "application/pdf"),
    )
    .await;
    serve(
        &server,
        "/loop",
        ResponseTemplate::new(302).insert_header("Location", "/loop"),
    )
    .await;
    serve(
        &server,
        "/script-only",
        html("<html><body><script>render()</script></body></html>"),
    )
    .await;

    let mut chunked = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    for _ in 0..4 {
        chunked.extend_from_slice(format!("200\r\n{}\r\n", "b".repeat(512)).as_bytes());
    }
    chunked.extend_from_slice(b"0\r\n\r\n");
    let streamed = raw_server(chunked).await;

    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);
        format!("http://{addr}/")
    };

    let config = capture::Config {
        timeout: Duration::from_millis(200),
        max_bytes: 1024,
        ..capture::Config::default()
    };
    let (client, _fake) = client(pool.clone(), "capture", config).await;
    let uri = server.uri();
    let cases = [
        (
            format!("{uri}/big"),
            "too_large",
            "The page is too large to read.",
        ),
        (streamed, "too_large", "The page is too large to read."),
        (
            format!("{uri}/slow"),
            "timeout",
            "The page took too long to answer.",
        ),
        (
            format!("{uri}/missing"),
            "http_status",
            "The site answered with HTTP 404.",
        ),
        (
            format!("{uri}/file.pdf"),
            "not_html",
            "The link isn't a web page.",
        ),
        (closed, "unreachable", "Couldn't connect to the site."),
        (
            format!("{uri}/loop"),
            "unreachable",
            "Couldn't connect to the site.",
        ),
        (
            format!("{uri}/script-only"),
            "empty_page",
            "The page has no readable text.",
        ),
    ];
    for (url, reason, message) in cases {
        let response = capture_url(&client, &url).await;
        assert_eq!(response.status(), Status::BadGateway, "{url}");
        let body: Value = response.into_json().await.expect("json");
        assert_eq!(body, json!({"reason": reason, "message": message}), "{url}");
    }
    assert_eq!(count(&pool, "jobs").await, 0);
}

#[sqlx::test]
async fn invalid_urls_are_rejected(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(html(JSONLD_BASIC))
        .expect(0)
        .mount(&server)
        .await;
    let (client, _fake) = client(pool.clone(), "capture", capture::Config::default()).await;
    let addr = server.address();
    let long = format!("{}/{}", server.uri(), "a".repeat(2048));
    let cases = [
        (json!({}), "Required"),
        (json!({"url": null}), "Required"),
        (json!({"url": "   "}), "Required"),
        (json!({"url": long}), "At most 2048 characters"),
        (
            json!({"url": "javascript:alert(1)"}),
            "Must be an http or https URL",
        ),
        (
            json!({"url": format!("ftp://{addr}/job")}),
            "Must be an http or https URL",
        ),
        (json!({"url": "/jobs/1"}), "Must be an http or https URL"),
        (
            json!({"url": format!("http://user:pass@{addr}/job")}),
            "Must not contain a username or password",
        ),
        (
            json!({"url": format!("http://user@{addr}/job")}),
            "Must not contain a username or password",
        ),
    ];
    for (body, message) in cases {
        let response = post(&client, &body).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{body}");
        let got: Value = response.into_json().await.expect("json");
        assert_eq!(
            got,
            json!({"errors": [{"field": "url", "message": message}]}),
            "{body}"
        );
    }
    assert_eq!(count(&pool, "jobs").await, 0);
}

#[sqlx::test]
async fn guards_refuse_foreign_requests(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(html(JSONLD_NONE))
        .expect(0)
        .mount(&server)
        .await;
    let (client, _fake) = client(pool.clone(), "capture", capture::Config::default()).await;
    let url = format!("{}/job", server.uri());
    let json = json!({ "url": url }).to_string();

    let foreign_host = client
        .post("/captures")
        .header(Header::new("Host", "evil.example"))
        .header(ContentType::JSON)
        .body(&json)
        .dispatch()
        .await;
    assert_eq!(foreign_host.status(), Status::Forbidden);

    let cross_site = client
        .post("/captures")
        .header(Header::new("Host", common::HOST))
        .header(Header::new("Sec-Fetch-Site", "cross-site"))
        .header(ContentType::JSON)
        .body(&json)
        .dispatch()
        .await;
    assert_eq!(cross_site.status(), Status::Forbidden);

    let form = client
        .post("/captures")
        .header(Header::new("Host", common::HOST))
        .header(ContentType::Form)
        .body(format!("url={url}"))
        .dispatch()
        .await;
    assert_eq!(form.status(), Status::Forbidden);

    let multipart = client
        .post("/captures")
        .header(Header::new("Host", common::HOST))
        .header(Header::new(
            "Content-Type",
            "multipart/form-data; boundary=x",
        ))
        .body(format!(
            "--x\r\nContent-Disposition: form-data; name=\"url\"\r\n\r\n{url}\r\n--x--\r\n"
        ))
        .dispatch()
        .await;
    assert_eq!(multipart.status(), Status::Forbidden);

    let plain = client
        .post("/captures")
        .header(Header::new("Host", common::HOST))
        .header(ContentType::Plain)
        .body(&json)
        .dispatch()
        .await;
    assert_eq!(plain.status(), Status::UnsupportedMediaType);

    assert_eq!(count(&pool, "jobs").await, 0);
}

#[sqlx::test]
async fn public_to_loopback_redirect_is_refused(pool: PgPool) {
    let origin = MockServer::start().await;
    let target = MockServer::start().await;
    Mock::given(any())
        .respond_with(html(JSONLD_BASIC))
        .expect(0)
        .mount(&target)
        .await;
    serve(
        &origin,
        "/start",
        ResponseTemplate::new(302).insert_header("Location", format!("{}/job", target.uri())),
    )
    .await;
    let config = capture::Config {
        treat_as_public: vec![*origin.address()],
        ..capture::Config::default()
    };
    let (client, _fake) = client(pool.clone(), "capture", config).await;

    let response = capture_url(&client, &format!("{}/start", origin.uri())).await;

    assert_eq!(response.status(), Status::UnprocessableEntity);
    let body: Value = response.into_json().await.expect("json");
    assert_eq!(
        body,
        json!({"errors": [{"field": "url", "message": REDIRECT_REFUSED}]})
    );
    assert_eq!(count(&pool, "jobs").await, 0);
}

#[sqlx::test]
async fn loopback_to_loopback_redirect_is_followed(pool: PgPool) {
    let origin = MockServer::start().await;
    let target = MockServer::start().await;
    serve(&target, "/job", html(JSONLD_BASIC)).await;
    serve(
        &origin,
        "/start",
        ResponseTemplate::new(302).insert_header("Location", format!("{}/job", target.uri())),
    )
    .await;
    let (client, _fake) = client(pool.clone(), "capture", capture::Config::default()).await;

    let url = format!("{}/start", origin.uri());
    let response = capture_url(&client, &url).await;

    assert_eq!(response.status(), Status::Ok);
    let body: Value = response.into_json().await.expect("json");
    assert_eq!(body["method"], "json_ld");
    assert_eq!(body["fields"]["company"], "Acme Corp");
    assert_eq!(
        body["fields"]["url"], url,
        "the submitted URL, not the final one"
    );
}
