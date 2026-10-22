//! `POST /dev/claude/ping` against `fake-claude.sh`.

mod common;

use std::time::Duration;

use rocket::http::Status;
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn client(pool: PgPool, mode: &str) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(10));
    let rocket = waypoint_api::rocket(rocket::Config::figment(), pool, fake.runner.clone());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

#[sqlx::test]
async fn ping_returns_text_and_usage(pool: PgPool) {
    let (client, _fake) = client(pool.clone(), "success").await;
    let response = client.post("/dev/claude/ping").dispatch().await;

    assert_eq!(response.status(), Status::Ok);
    let body: Value = response.into_json().await.expect("json body");
    assert_eq!(body["text"], "pong");
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(body["input_tokens"], 2);
    assert_eq!(body["output_tokens"], 4);
    assert_eq!(body["cache_read_input_tokens"], 649);
    assert_eq!(body["cache_creation_input_tokens"], 3327);
    assert_eq!(body["cost_usd"], 0.00067409);
    assert!(body["duration_ms"].is_u64(), "{body}");

    let (id, purpose, status): (i64, String, String) =
        sqlx::query_as("SELECT id, purpose, status FROM claude_calls")
            .fetch_one(&pool)
            .await
            .expect("one logged row");
    assert_eq!(body["log_id"], id);
    assert_eq!((purpose.as_str(), status.as_str()), ("dev.ping", "ok"));
}

#[sqlx::test]
async fn ping_maps_usage_limit_to_429(pool: PgPool) {
    let (client, _fake) = client(pool, "usage_limit").await;
    let response = client.post("/dev/claude/ping").dispatch().await;

    assert_eq!(response.status(), Status::TooManyRequests);
    let body: Value = response.into_json().await.expect("json body");
    assert_eq!(
        body,
        json!({
            "error": "usage_limit",
            "message": "You've hit your limit · resets 5pm (America/New_York)",
        })
    );
}
