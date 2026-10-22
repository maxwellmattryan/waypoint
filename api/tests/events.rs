//! `GET /events`, `GET /jobs/<id>` and `POST /dev/jobs/test-prompt` over HTTP, against
//! `fake-claude.sh`.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::Value;
use sqlx::PgPool;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};

async fn client(pool: PgPool, mode: &str) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(10));
    let rocket = common::app(pool, fake.runner.clone(), common::fast_config());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The next SSE message as `(event, data)`, skipping heartbeat comments.
async fn next_message(reader: &mut (impl AsyncBufRead + Unpin)) -> (String, Value) {
    let mut event = None;
    let mut data = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await.expect("read the stream");
        assert!(read > 0, "the stream ended");
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if let (Some(event), Some(data)) = (event.take(), data.take()) {
                return (event, data);
            }
        } else if let Some(name) = line.strip_prefix("event:") {
            event = Some(name.trim().to_owned());
        } else if let Some(json) = line.strip_prefix("data:") {
            data = Some(serde_json::from_str(json.trim()).expect("data is JSON"));
        }
    }
}

async fn job_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(pool)
        .await
        .expect("count jobs")
}

#[sqlx::test]
async fn test_prompt_round_trips_over_sse(pool: PgPool) {
    let (client, _fake) = client(pool, "success").await;
    tokio::time::timeout(Duration::from_secs(10), async {
        let stream: LocalResponse<'_> = client.get("/events").header(host()).dispatch().await;
        assert_eq!(stream.status(), Status::Ok);
        assert_eq!(stream.content_type(), Some(ContentType::EventStream));
        let mut stream = BufReader::new(stream);

        let (event, data) = next_message(&mut stream).await;
        assert_eq!(event, "app.connected");
        assert_eq!(data, serde_json::json!({"type": "app.connected"}));

        let enqueued = client
            .post("/dev/jobs/test-prompt")
            .header(host())
            .header(Header::new("X-Waypoint-Dev", "1"))
            .dispatch()
            .await;
        assert_eq!(enqueued.status(), Status::Accepted);
        let body: Value = enqueued.into_json().await.expect("json body");
        let id = body["job_id"].as_i64().expect("job_id");

        let mut seen = Vec::new();
        while seen.last().map(String::as_str) != Some("job.succeeded") {
            let (event, data) = next_message(&mut stream).await;
            assert_eq!(data["type"], event.as_str());
            if data["job"]["id"] == id {
                seen.push(event);
            }
        }
        assert_eq!(seen, ["job.queued", "job.started", "job.succeeded"]);

        let job = client
            .get(format!("/jobs/{id}"))
            .header(host())
            .dispatch()
            .await;
        assert_eq!(job.status(), Status::Ok);
        let job: Value = job.into_json().await.expect("json body");
        assert_eq!(job["id"], id);
        assert_eq!(job["status"], "succeeded");
        assert_eq!(job["result"]["text"], "pong");
        assert!(
            job.get("payload").is_none(),
            "the payload is not echoed: {job}"
        );

        let missing = client.get("/jobs/999999").header(host()).dispatch().await;
        assert_eq!(missing.status(), Status::NotFound);
    })
    .await
    .expect("the round trip finishes within 10s");
}

#[sqlx::test]
async fn dev_test_prompt_rejects_unmarked_and_cross_site(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "success").await;

    let bare = client
        .post("/dev/jobs/test-prompt")
        .header(host())
        .dispatch()
        .await;
    assert_eq!(bare.status(), Status::Forbidden);

    for site in ["cross-site", "same-site"] {
        let foreign = client
            .post("/dev/jobs/test-prompt")
            .header(host())
            .header(Header::new("X-Waypoint-Dev", "1"))
            .header(Header::new("Sec-Fetch-Site", site))
            .dispatch()
            .await;
        assert_eq!(foreign.status(), Status::Forbidden, "{site}");
    }

    assert_eq!(
        job_count(&pool).await,
        0,
        "no rejected request enqueued a job"
    );
}
