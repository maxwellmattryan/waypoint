use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use waypoint_api::claude::ClaudeRunner;

fn local_host() -> Header<'static> {
    Header::new("Host", "127.0.0.1")
}

async fn client(pool: PgPool) -> Client {
    // The health check never calls the runner.
    let claude = ClaudeRunner::new("claude", Duration::from_secs(1));
    let rocket = waypoint_api::rocket(
        rocket::Config::figment(),
        pool,
        claude,
        waypoint_api::jobs::Config::default(),
        waypoint_api::google::Config::default(),
        waypoint_api::capture::Config::default(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
}

#[sqlx::test]
async fn health_ok_when_db_reachable(pool: PgPool) {
    let client = client(pool).await;
    let response = client.get("/health").header(local_host()).dispatch().await;

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.content_type(), Some(ContentType::JSON));
    let body: Value = response.into_json().await.expect("json body");
    assert_eq!(body, json!({"status": "ok", "db": "ok"}));
}

#[rocket::async_test]
async fn health_degraded_when_db_unreachable() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        // Nothing listens on port 1, so every acquire fails without needing a database.
        .connect_lazy("postgres://127.0.0.1:1/none")
        .expect("valid url");
    let client = client(pool).await;
    let response = client.get("/health").header(local_host()).dispatch().await;

    assert_eq!(response.status(), Status::ServiceUnavailable);
    assert_eq!(response.content_type(), Some(ContentType::JSON));
    let body: Value = response.into_json().await.expect("json body");
    assert_eq!(body, json!({"status": "degraded", "db": "unavailable"}));
}
