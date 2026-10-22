//! The Host-header allowlist on every mounted route.

mod common;

use std::time::Duration;

use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use sqlx::PgPool;

/// `route`'s path with every dynamic segment (`<id>`) filled with `1`.
fn concrete_path(route: &rocket::Route) -> String {
    route
        .uri
        .path()
        .split('/')
        .map(|segment| {
            if segment.starts_with('<') {
                "1"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[sqlx::test]
async fn every_route_rejects_foreign_or_missing_host(pool: PgPool) {
    let fake = common::fake("success", Duration::from_secs(10));
    let rocket = common::app(pool.clone(), fake.runner.clone(), common::fast_config());
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let routes: Vec<(rocket::http::Method, String)> = client
        .rocket()
        .routes()
        .map(|route| (route.method, concrete_path(route)))
        .collect();
    for expected in ["/health", "/events", "/jobs/1", "/dev/jobs/test-prompt"] {
        assert!(
            routes.iter().any(|(_, path)| path == expected),
            "{expected} is mounted: {routes:?}"
        );
    }

    for (method, path) in &routes {
        for host in [Some("evil.example"), None] {
            let mut request = client
                .req(*method, path.as_str())
                .header(Header::new("X-Waypoint-Dev", "1"));
            if let Some(host) = host {
                request = request.header(Header::new("Host", host));
            }
            let response = request.dispatch().await;
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "{method} {path} with Host {host:?}"
            );
        }
    }

    for table in ["jobs", "claude_calls"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 0, "a rejected request wrote to {table}");
    }
}
