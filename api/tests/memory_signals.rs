//! The learning-signals table and its one route: the table's CHECKs, and `GET
//! /memory-signals` listing newest first, filtering by kind and paging. The decisions that
//! write signals are tested in `approvals.rs`.

use std::time::Duration;

use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::claude::ClaudeRunner;

fn host() -> Header<'static> {
    Header::new("Host", "127.0.0.1")
}

async fn client(pool: PgPool) -> Client {
    // The signals route never calls the runner.
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

/// `(status, JSON body or null)` of `GET path`.
async fn get(client: &Client, path: &str) -> (Status, Value) {
    let response = client.get(path.to_owned()).header(host()).dispatch().await;
    let status = response.status();
    let body = response.into_json().await.unwrap_or(Value::Null);
    (status, body)
}

/// The listed ids of `GET path`, which must succeed.
async fn ids(client: &Client, path: &str) -> Vec<i64> {
    let (status, body) = get(client, path).await;
    assert_eq!(status, Status::Ok, "GET {path}: {body}");
    body.as_array()
        .expect("an array")
        .iter()
        .map(|signal| signal["id"].as_i64().expect("id"))
        .collect()
}

/// Ids `to` down to `from`, newest first, keeping those `keep` accepts.
fn descending(from: i64, to: i64, keep: impl Fn(i64) -> bool) -> Vec<i64> {
    (from..=to).rev().filter(|id| keep(*id)).collect()
}

/// Signals 1..=205: `id % 3` is 1 for a rejection, 2 for a dismissal (with a reason) and 0
/// for an approval.
async fn seed(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO memory_signals \
         (kind, run_id, pending_action_id, review_item_id, subject, drafted, approved, changes, reason) \
         SELECT kind, 7, CASE WHEN kind <> 'dismissal' THEN i END, \
         CASE WHEN kind = 'dismissal' THEN i END, 'send', jsonb_build_object('i', i), \
         CASE WHEN kind = 'approval' THEN jsonb_build_object('i', i) END, \
         CASE WHEN kind = 'approval' THEN '[]'::jsonb END, \
         CASE WHEN kind = 'dismissal' THEN 'Wrong role' END \
         FROM (SELECT i, (ARRAY['approval', 'rejection', 'dismissal'])[i % 3 + 1] AS kind \
         FROM generate_series(1, 205) AS i) AS rows ORDER BY i",
    )
    .execute(pool)
    .await
    .expect("seed signals");
}

#[sqlx::test]
async fn lists_filters_and_pages(pool: PgPool) {
    seed(&pool).await;
    sqlx::query(
        "UPDATE memory_signals SET approved = '{\"m\": \"b\"}', \
         changes = '[{\"op\": \"replace\", \"path\": \"/m\", \"before\": \"a\", \"after\": \"b\"}]' \
         WHERE id = 3",
    )
    .execute(&pool)
    .await
    .expect("edit an approval");
    let client = client(pool).await;

    // Newest first, 50 by default, and `limit` clamped to 1..=200.
    let any = |_| true;
    assert_eq!(
        ids(&client, "/memory-signals").await,
        descending(156, 205, any)
    );
    assert_eq!(ids(&client, "/memory-signals?limit=0").await, [205]);
    assert_eq!(ids(&client, "/memory-signals?limit=-3").await, [205]);
    assert_eq!(ids(&client, "/memory-signals?limit=2").await, [205, 204]);
    assert_eq!(
        ids(&client, "/memory-signals?limit=500").await,
        descending(6, 205, any)
    );

    // `before` pages, and `kind` filters, alone or together; blank means any.
    assert_eq!(
        ids(&client, "/memory-signals?before=10").await,
        descending(1, 9, any)
    );
    assert_eq!(
        ids(&client, "/memory-signals?kind=rejection&limit=200").await,
        descending(1, 205, |id| id % 3 == 1)
    );
    assert_eq!(
        ids(&client, "/memory-signals?kind=dismissal&before=12&limit=2").await,
        [11, 8]
    );
    assert_eq!(
        ids(&client, "/memory-signals?kind=%20&limit=3").await,
        [205, 204, 203]
    );

    // Every column comes back, `changes` decoded.
    let (_, page) = get(&client, "/memory-signals?before=4").await;
    let page = page.as_array().expect("an array");
    let (approval, dismissal, rejection) = (&page[0], &page[1], &page[2]);
    assert_eq!(approval["kind"], "approval");
    assert_eq!(approval["pending_action_id"], 3);
    assert_eq!(approval["review_item_id"], Value::Null);
    assert_eq!(approval["run_id"], 7);
    assert_eq!(approval["subject"], "send");
    assert_eq!(approval["drafted"], json!({"i": 3}));
    assert_eq!(approval["approved"], json!({"m": "b"}));
    assert_eq!(
        approval["changes"],
        json!([{"op": "replace", "path": "/m", "before": "a", "after": "b"}])
    );
    assert_eq!(approval["reason"], Value::Null);
    assert!(approval["created_at"].is_string());
    assert_eq!(dismissal["kind"], "dismissal");
    assert_eq!(dismissal["review_item_id"], 2);
    assert_eq!(dismissal["reason"], "Wrong role");
    assert_eq!(dismissal["changes"], Value::Null);
    assert_eq!(rejection["kind"], "rejection");
    assert_eq!(rejection["approved"], Value::Null);

    let (status, body) = get(&client, "/memory-signals?kind=accept").await;
    assert_eq!(status, Status::UnprocessableEntity);
    assert_eq!(
        body,
        json!({"errors": [{"field": "kind", "message": "Unknown kind"}]})
    );
}

#[sqlx::test]
async fn refuses_a_non_local_host(pool: PgPool) {
    seed(&pool).await;
    let client = client(pool).await;
    let response = client
        .get("/memory-signals")
        .header(Header::new("Host", "evil.example"))
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Forbidden);
}

#[sqlx::test]
async fn checks_refuse_mismatched_rows(pool: PgPool) {
    const COLUMNS: &str = "kind, run_id, pending_action_id, review_item_id, subject, drafted, \
                           approved, changes, reason";
    for (what, values) in [
        (
            "an unknown kind",
            "'accept', 1, 1, NULL, 's', '{}', NULL, NULL, NULL",
        ),
        (
            "an approval without changes",
            "'approval', 1, 1, NULL, 's', '{}', '{}', NULL, NULL",
        ),
        (
            "an approval whose changes aren't an array",
            "'approval', 1, 1, NULL, 's', '{}', '{}', '{}', NULL",
        ),
        (
            "an approval without the approved input",
            "'approval', 1, 1, NULL, 's', '{}', NULL, '[]', NULL",
        ),
        (
            "an approval without a pending action",
            "'approval', 1, NULL, NULL, 's', '{}', '{}', '[]', NULL",
        ),
        (
            "an approval with a reason",
            "'approval', 1, 1, NULL, 's', '{}', '{}', '[]', 'why'",
        ),
        (
            "a rejection with a reason",
            "'rejection', 1, 1, NULL, 's', '{}', NULL, NULL, 'why'",
        ),
        (
            "a rejection with changes",
            "'rejection', 1, 1, NULL, 's', '{}', NULL, '[]', NULL",
        ),
        (
            "a rejection with a review item",
            "'rejection', 1, 1, 1, 's', '{}', NULL, NULL, NULL",
        ),
        (
            "a dismissal with a pending action",
            "'dismissal', 1, 1, 1, 's', '{}', NULL, NULL, NULL",
        ),
        (
            "a dismissal without a review item",
            "'dismissal', 1, NULL, NULL, 's', '{}', NULL, NULL, NULL",
        ),
        (
            "a dismissal with an approved input",
            "'dismissal', 1, NULL, 1, 's', '{}', '{}', NULL, NULL",
        ),
        (
            "an empty reason",
            "'dismissal', 1, NULL, 1, 's', '{}', NULL, NULL, ''",
        ),
        (
            "a reason over 1000 characters",
            "'dismissal', 1, NULL, 1, 's', '{}', NULL, NULL, repeat('x', 1001)",
        ),
    ] {
        let sql = format!("INSERT INTO memory_signals ({COLUMNS}) VALUES ({values})");
        assert!(
            sqlx::query(&sql).execute(&pool).await.is_err(),
            "the table refuses {what}"
        );
    }

    for values in [
        "'approval', 1, 1, NULL, 's', '{}', '{}', '[]', NULL",
        "'rejection', 1, 2, NULL, 's', '{}', NULL, NULL, NULL",
        "'dismissal', 1, NULL, 1, 's', '{}', NULL, NULL, NULL",
        "'dismissal', 1, NULL, 2, 's', '{}', NULL, NULL, repeat('x', 1000)",
    ] {
        let sql = format!("INSERT INTO memory_signals ({COLUMNS}) VALUES ({values})");
        sqlx::query(&sql).execute(&pool).await.expect(&sql);
    }

    // One signal per decision.
    for (what, values) in [
        (
            "a second signal for a pending action",
            "'rejection', 1, 1, NULL, 's', '{}', NULL, NULL, NULL",
        ),
        (
            "a second signal for a review item",
            "'dismissal', 1, NULL, 1, 's', '{}', NULL, NULL, NULL",
        ),
    ] {
        let sql = format!("INSERT INTO memory_signals ({COLUMNS}) VALUES ({values})");
        assert!(
            sqlx::query(&sql).execute(&pool).await.is_err(),
            "the table refuses {what}"
        );
    }
}
