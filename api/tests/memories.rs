//! The memories store and routes: the table's CHECKs, create, read, list with filters and
//! full-text search, update, confirm and delete, their validation and the guards on every
//! changing route.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::memories::{
    self, MemoryInput, MemoryKind, MemorySource, MemoryStatus, MemoryTopic, NewMemory, SourceKind,
};

fn host() -> Header<'static> {
    Header::new("Host", "127.0.0.1")
}

async fn client(pool: PgPool) -> Client {
    // No memory route calls the runner.
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

async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM memories")
        .fetch_one(pool)
        .await
        .expect("count memories")
}

/// Sends `body` as JSON with `method` to `path`.
async fn send<'c>(
    client: &'c Client,
    method: Method,
    path: &str,
    body: &Value,
) -> LocalResponse<'c> {
    client
        .req(method, path.to_string())
        .header(host())
        .header(ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await
}

async fn json_body(response: LocalResponse<'_>) -> Value {
    response.into_json().await.expect("json body")
}

async fn get<'c>(client: &'c Client, path: &str) -> LocalResponse<'c> {
    client.get(path.to_string()).header(host()).dispatch().await
}

async fn post<'c>(client: &'c Client, path: &str) -> LocalResponse<'c> {
    client
        .post(path.to_string())
        .header(host())
        .dispatch()
        .await
}

/// `GET /memories` with `query`, expecting 200; returns the contents in order.
async fn list(client: &Client, query: &str) -> Vec<String> {
    let response = get(client, &format!("/memories{query}")).await;
    assert_eq!(response.status(), Status::Ok, "{query}");
    json_body(response)
        .await
        .as_array()
        .expect("a list")
        .iter()
        .map(|m| m["content"].as_str().unwrap().to_string())
        .collect()
}

/// Stores a memory through [`memories::insert`], as later workflows will.
async fn seed(
    pool: &PgPool,
    kind: MemoryKind,
    topic: MemoryTopic,
    content: &str,
    tags: &[&str],
    status: MemoryStatus,
    source: MemorySource,
) -> i64 {
    let fields = memories::validate(&MemoryInput {
        kind,
        topic,
        content: content.into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
    })
    .expect("valid memory");
    memories::insert(
        pool,
        &NewMemory {
            fields,
            status,
            source,
        },
    )
    .await
    .expect("insert a memory")
    .id
}

/// Moves every timestamp of row `id` `minutes` into the past, so a later `now()` is
/// strictly greater and rows can be ordered on purpose.
async fn age(pool: &PgPool, id: i64, minutes: i32) {
    sqlx::query(
        "UPDATE memories SET created_at = now() - make_interval(mins => $2), \
         updated_at = now() - make_interval(mins => $2), \
         confirmed_at = CASE WHEN confirmed_at IS NULL THEN NULL \
         ELSE now() - make_interval(mins => $2) END WHERE id = $1",
    )
    .bind(id)
    .bind(minutes)
    .execute(pool)
    .await
    .expect("age row");
}

fn id(memory: &Value) -> i64 {
    memory["id"].as_i64().expect("id")
}

fn time(value: &Value) -> DateTime<Utc> {
    value
        .as_str()
        .expect("a timestamp")
        .parse()
        .expect("RFC 3339")
}

fn errors(body: &Value) -> Vec<(String, String)> {
    body["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .map(|e| {
            (
                e["field"].as_str().unwrap().to_string(),
                e["message"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn pair(field: &str, message: &str) -> (String, String) {
    (field.to_string(), message.to_string())
}

#[sqlx::test]
async fn checks_reject_bad_rows(pool: PgPool) {
    for (what, sql) in [
        (
            "a topic of another kind",
            "INSERT INTO memories (kind, topic, content) VALUES ('fact', 'comp_floor', 'x')",
        ),
        (
            "an unknown kind",
            "INSERT INTO memories (kind, topic, content) VALUES ('wish', 'other', 'x')",
        ),
        (
            "an unknown status",
            "INSERT INTO memories (kind, topic, content, status) \
             VALUES ('fact', 'skill', 'x', 'archived')",
        ),
        (
            "an unknown source_kind",
            "INSERT INTO memories (kind, topic, content, source_kind) \
             VALUES ('fact', 'skill', 'x', 'twitter')",
        ),
    ] {
        assert!(
            sqlx::query(sql).execute(&pool).await.is_err(),
            "the table refuses {what}"
        );
    }

    let (status, source): (String, String) = sqlx::query_as(
        "INSERT INTO memories (kind, topic, content) VALUES ('preference', 'comp_floor', 'x') \
         RETURNING status, source_kind",
    )
    .fetch_one(&pool)
    .await
    .expect("a valid row is stored");
    assert_eq!((status.as_str(), source.as_str()), ("confirmed", "manual"));
    assert_eq!(count(&pool).await, 1);
}

#[sqlx::test]
async fn create_then_get_round_trips(pool: PgPool) {
    let client = client(pool.clone()).await;
    let response = send(
        &client,
        Method::Post,
        "/memories",
        &json!({
            "kind": "preference",
            "topic": "location",
            "content": "  Remote, or hybrid in Berlin\nNo relocation  ",
            "tags": [" Remote ", "remote", "Berlin   Area", "", "REMOTE"],
            // Ignored: the HTTP API never sets status or provenance.
            "status": "proposed",
            "source_kind": "email",
            "source_ref": "abc",
            "source_label": "Forged",
        }),
    )
    .await;
    assert_eq!(response.status(), Status::Created);
    let created = json_body(response).await;
    assert_eq!(created["kind"], "preference");
    assert_eq!(created["topic"], "location");
    assert_eq!(
        created["content"],
        "Remote, or hybrid in Berlin\nNo relocation"
    );
    assert_eq!(created["tags"], json!(["remote", "berlin area"]));
    assert_eq!(created["status"], "confirmed");
    assert!(created["confirmed_at"].is_string(), "{created}");
    assert_eq!(created["source_kind"], "manual");
    assert_eq!(created["source_ref"], Value::Null);
    assert_eq!(created["source_label"], Value::Null);
    assert_eq!(created["created_at"], created["updated_at"]);

    let response = get(&client, &format!("/memories/{}", id(&created))).await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(json_body(response).await, created);

    let response = get(&client, &format!("/memories/{}", id(&created) + 1)).await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn invalid_input_is_rejected(pool: PgPool) {
    let client = client(pool.clone()).await;
    let existing = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Skill,
        "Rust",
        &[],
        MemoryStatus::Confirmed,
        MemorySource::manual(),
    )
    .await;
    let one = format!("/memories/{existing}");
    let many: Vec<String> = (0..21).map(|i| format!("tag {i}")).collect();
    let long_tag = "x".repeat(51);

    for (method, path) in [(Method::Post, "/memories"), (Method::Put, one.as_str())] {
        let response = send(
            &client,
            method,
            path,
            &json!({"kind": "fact", "topic": "comp_floor", "content": "   ", "tags": many}),
        )
        .await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{method}");
        assert_eq!(
            errors(&json_body(response).await),
            vec![
                pair("content", "Required"),
                pair("topic", "Not a topic for this kind"),
                pair("tags", "At most 20 tags"),
            ],
            "{method}"
        );

        let response = send(
            &client,
            method,
            path,
            &json!({"kind": "answer", "topic": "notice_period", "content": "x".repeat(5001), "tags": [long_tag]}),
        )
        .await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{method}");
        assert_eq!(
            errors(&json_body(response).await),
            vec![
                pair("content", "At most 5000 characters"),
                pair("tags", "Each tag at most 50 characters"),
            ],
            "{method}"
        );

        let response = send(
            &client,
            method,
            path,
            &json!({"kind": "fact", "topic": "skill"}),
        )
        .await;
        assert_eq!(
            response.status(),
            Status::UnprocessableEntity,
            "missing content"
        );
        assert_eq!(
            errors(&json_body(response).await),
            vec![pair("content", "Required")]
        );

        for body in [
            json!({"kind": "wish", "topic": "other", "content": "x"}),
            json!({"kind": "fact", "topic": "hobby", "content": "x"}),
            json!({"topic": "other", "content": "x"}),
            json!({"kind": "fact", "topic": "skill", "content": "x", "tags": "rust"}),
        ] {
            let response = send(&client, method, path, &body).await;
            assert_eq!(response.status(), Status::BadRequest, "{method} {body}");
        }
        let response = client
            .req(method, path)
            .header(host())
            .header(ContentType::JSON)
            .body("{\"kind\": ")
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest, "{method} malformed");
    }

    // Twenty tags of fifty characters, and duplicates past twenty, are fine.
    let tags: Vec<String> = (0..20)
        .map(|i| format!("{i:02}{}", "y".repeat(48)))
        .chain(["00".to_string() + &"Y".repeat(48)])
        .collect();
    let response = send(
        &client,
        Method::Post,
        "/memories",
        &json!({"kind": "fact", "topic": "other", "content": "x", "tags": tags}),
    )
    .await;
    assert_eq!(response.status(), Status::Created);
    assert_eq!(
        json_body(response).await["tags"].as_array().unwrap().len(),
        20
    );

    assert_eq!(count(&pool).await, 2, "nothing invalid was stored");
    let response = get(&client, &one).await;
    assert_eq!(
        json_body(response).await["content"],
        "Rust",
        "nothing was changed"
    );
}

#[sqlx::test]
async fn list_filters_and_orders(pool: PgPool) {
    let client = client(pool.clone()).await;
    let manual = MemorySource::manual;
    // Oldest first, so the expected order is the reverse of this list.
    let rows = [
        (
            MemoryKind::Fact,
            MemoryTopic::Skill,
            "Rust",
            &["backend"][..],
            MemoryStatus::Confirmed,
        ),
        (
            MemoryKind::Preference,
            MemoryTopic::Location,
            "Remote only",
            &["remote", "backend"][..],
            MemoryStatus::Proposed,
        ),
        (
            MemoryKind::Answer,
            MemoryTopic::NoticePeriod,
            "Four weeks",
            &[][..],
            MemoryStatus::Confirmed,
        ),
        (
            MemoryKind::Preference,
            MemoryTopic::Seniority,
            "Staff or above",
            &["backend"][..],
            MemoryStatus::Confirmed,
        ),
    ];
    let mut minutes = 40;
    for (kind, topic, content, tags, status) in rows {
        let id = seed(&pool, kind, topic, content, tags, status, manual()).await;
        age(&pool, id, minutes).await;
        minutes -= 10;
    }
    // Two rows with the same `updated_at` order by `id` descending.
    let tie_a = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Education,
        "BSc",
        &[],
        MemoryStatus::Confirmed,
        manual(),
    )
    .await;
    let tie_b = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Education,
        "MSc",
        &[],
        MemoryStatus::Confirmed,
        manual(),
    )
    .await;
    sqlx::query(
        "UPDATE memories SET updated_at = now() - interval '5 minutes' WHERE id IN ($1, $2)",
    )
    .bind(tie_a)
    .bind(tie_b)
    .execute(&pool)
    .await
    .expect("tie rows");

    assert_eq!(
        list(&client, "").await,
        [
            "MSc",
            "BSc",
            "Staff or above",
            "Four weeks",
            "Remote only",
            "Rust"
        ]
    );
    assert_eq!(
        list(&client, "?kind=&status=&tag=%20&q=%20").await,
        list(&client, "").await,
        "blank parameters count as absent"
    );
    assert_eq!(
        list(&client, "?kind=preference").await,
        ["Staff or above", "Remote only"]
    );
    assert_eq!(list(&client, "?status=proposed").await, ["Remote only"]);
    assert_eq!(
        list(&client, "?tag=%20BACKEND%20").await,
        ["Staff or above", "Remote only", "Rust"],
        "the tag is normalised"
    );
    assert_eq!(
        list(&client, "?kind=preference&status=confirmed&tag=backend").await,
        ["Staff or above"]
    );
    assert!(list(&client, "?kind=answer&tag=backend").await.is_empty());

    for (query, field) in [("?kind=wish", "kind"), ("?status=archived", "status")] {
        let response = get(&client, &format!("/memories{query}")).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{query}");
        let failed: Vec<String> = errors(&json_body(response).await)
            .into_iter()
            .map(|(field, _)| field)
            .collect();
        assert_eq!(failed, [field], "{query}");
    }
}

#[sqlx::test]
async fn search_matches_content_tags_and_topic(pool: PgPool) {
    let client = client(pool.clone()).await;
    let manual = MemorySource::manual;
    let confirmed = MemoryStatus::Confirmed;
    seed(
        &pool,
        MemoryKind::Preference,
        MemoryTopic::Location,
        "I want to work remotely",
        &[],
        confirmed,
        manual(),
    )
    .await;
    seed(
        &pool,
        MemoryKind::Answer,
        MemoryTopic::SalaryExpectation,
        "180k base plus equity",
        &[],
        confirmed,
        manual(),
    )
    .await;
    seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Skill,
        "Ten years of Go",
        &["distributed systems"],
        confirmed,
        manual(),
    )
    .await;
    let both = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Position,
        "Led the Kafka and Postgres migration",
        &[],
        confirmed,
        manual(),
    )
    .await;
    let one = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Other,
        "Ran Postgres upgrades",
        &[],
        confirmed,
        manual(),
    )
    .await;
    // The weaker match is the newer one, so only the rank can put `both` first.
    age(&pool, both, 10).await;
    age(&pool, one, 1).await;

    assert_eq!(
        list(&client, "?q=remote").await,
        ["I want to work remotely"],
        "stemmed content"
    );
    assert_eq!(
        list(&client, "?q=Systems").await,
        ["Ten years of Go"],
        "tag words"
    );
    assert_eq!(
        list(&client, "?q=salary").await,
        ["180k base plus equity"],
        "topic words"
    );
    assert_eq!(
        list(&client, "?q=kafka%20or%20postgres").await,
        [
            "Led the Kafka and Postgres migration",
            "Ran Postgres upgrades"
        ],
        "more matching terms rank higher"
    );
    assert_eq!(
        list(&client, "?q=postgres&kind=fact&tag=").await.len(),
        2,
        "search combines with filters"
    );
    for query in ["?q=the%20and", "?q=zebra", "?q=%22unbalanced", "?q=-"] {
        assert!(list(&client, query).await.is_empty(), "{query}");
    }
}

#[sqlx::test]
async fn update_keeps_status_and_source(pool: PgPool) {
    let client = client(pool.clone()).await;
    let id = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Position,
        "Staff engineer at Acme",
        &["acme"],
        MemoryStatus::Proposed,
        MemorySource {
            kind: SourceKind::Resume,
            reference: Some("12".into()),
            label: Some("Resume · Staff Eng v3".into()),
        },
    )
    .await;
    age(&pool, id, 5).await;
    let path = format!("/memories/{id}");
    let before = json_body(get(&client, &path).await).await;

    let response = send(
        &client,
        Method::Put,
        &path,
        &json!({
            "kind": "preference",
            "topic": "target_role",
            "content": " Staff platform roles ",
            "tags": ["Platform"],
            "status": "confirmed",
            "source_kind": "manual",
        }),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    let updated = json_body(response).await;
    assert_eq!(updated["kind"], "preference");
    assert_eq!(updated["topic"], "target_role");
    assert_eq!(updated["content"], "Staff platform roles");
    assert_eq!(updated["tags"], json!(["platform"]));
    assert!(time(&updated["updated_at"]) > time(&before["updated_at"]));
    for kept in [
        "id",
        "status",
        "confirmed_at",
        "source_kind",
        "source_ref",
        "source_label",
        "created_at",
    ] {
        assert_eq!(updated[kept], before[kept], "{kept} is kept");
    }
    assert_eq!(updated["status"], "proposed");
    assert_eq!(updated["source_kind"], "resume");
    assert_eq!(updated["source_ref"], "12");
    assert_eq!(json_body(get(&client, &path).await).await, updated);

    let response = send(
        &client,
        Method::Put,
        &format!("/memories/{}", id + 1),
        &json!({"kind": "fact", "topic": "skill", "content": "x"}),
    )
    .await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn confirm_is_idempotent(pool: PgPool) {
    let client = client(pool.clone()).await;
    let id = seed(
        &pool,
        MemoryKind::Answer,
        MemoryTopic::WhyLeaving,
        "Looking for a larger scope",
        &[],
        MemoryStatus::Proposed,
        MemorySource {
            kind: SourceKind::Run,
            reference: Some("7".into()),
            label: None,
        },
    )
    .await;
    age(&pool, id, 5).await;
    let path = format!("/memories/{id}");
    let before = json_body(get(&client, &path).await).await;
    assert_eq!(before["confirmed_at"], Value::Null);

    let response = post(&client, &format!("{path}/confirm")).await;
    assert_eq!(response.status(), Status::Ok);
    let confirmed = json_body(response).await;
    assert_eq!(confirmed["status"], "confirmed");
    assert_eq!(confirmed["confirmed_at"], confirmed["updated_at"]);
    assert!(time(&confirmed["updated_at"]) > time(&before["updated_at"]));
    assert_eq!(confirmed["source_kind"], "run");
    assert_eq!(confirmed["content"], before["content"]);

    let response = post(&client, &format!("{path}/confirm")).await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        json_body(response).await,
        confirmed,
        "confirming again changes nothing"
    );

    let response = post(&client, &format!("/memories/{}/confirm", id + 1)).await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn delete_removes_the_row(pool: PgPool) {
    let client = client(pool.clone()).await;
    let id = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Skill,
        "Rust",
        &[],
        MemoryStatus::Confirmed,
        MemorySource::manual(),
    )
    .await;
    let path = format!("/memories/{id}");
    let response = client.delete(path.clone()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NoContent);
    assert_eq!(count(&pool).await, 0);
    assert_eq!(get(&client, &path).await.status(), Status::NotFound);

    let response = client.delete(path).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn guards_refuse_foreign_requests(pool: PgPool) {
    let client = client(pool.clone()).await;
    // Proposed, so a confirm that got through would show.
    let id = seed(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Skill,
        "Rust",
        &["systems"],
        MemoryStatus::Proposed,
        MemorySource::manual(),
    )
    .await;
    let one = format!("/memories/{id}");
    let confirm = format!("{one}/confirm");
    let before = json_body(get(&client, &one).await).await;
    let valid = json!({"kind": "fact", "topic": "other", "content": "Evil"}).to_string();

    let mutating: [(Method, &str, &str); 4] = [
        (Method::Post, "/memories", &valid),
        (Method::Put, &one, &valid),
        (Method::Post, &confirm, ""),
        (Method::Delete, &one, ""),
    ];
    for (method, path, body) in mutating {
        let response = client
            .req(method, path)
            .header(host())
            .header(ContentType::JSON)
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .body(body)
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "cross-site {method} {path}"
        );

        for form in [ContentType::Form, ContentType::FormData] {
            let response = client
                .req(method, path)
                .header(host())
                .header(form.clone())
                .body("kind=fact&topic=other&content=Evil")
                .dispatch()
                .await;
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "{form} {method} {path}"
            );
        }
    }

    // What any web page can auto-submit: a form POST Rocket turns into PUT or DELETE.
    for (path, override_) in [
        (&one, "put"),
        (&one, "delete"),
        (&confirm, "put"),
        (&confirm, "delete"),
    ] {
        let response = client
            .post(path.clone())
            .header(host())
            .header(ContentType::Form)
            .body(format!(
                "_method={override_}&kind=fact&topic=other&content=Evil"
            ))
            .dispatch()
            .await;
        if path == &confirm {
            // No PUT or DELETE route has this path, so the rewritten request finds none.
            assert_eq!(
                response.status(),
                Status::NotFound,
                "_method={override_} {path}"
            );
        } else {
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "_method={override_} {path}"
            );
        }
    }

    // A body that isn't declared JSON never reaches the handler.
    for (method, path) in [(Method::Post, "/memories"), (Method::Put, one.as_str())] {
        let response = client
            .req(method, path)
            .header(host())
            .header(ContentType::Plain)
            .body(valid.as_str())
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::UnsupportedMediaType,
            "text/plain {method} {path}"
        );
    }

    let evil = Header::new("Host", "evil.example");
    let all: [(Method, &str, &str); 6] = [
        (Method::Get, "/memories", ""),
        (Method::Post, "/memories", &valid),
        (Method::Get, &one, ""),
        (Method::Put, &one, &valid),
        (Method::Post, &confirm, ""),
        (Method::Delete, &one, ""),
    ];
    for (method, path, body) in all {
        let response = client
            .req(method, path)
            .header(evil.clone())
            .header(ContentType::JSON)
            .body(body)
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "evil host {method} {path}"
        );
    }

    assert_eq!(count(&pool).await, 1, "nothing was created or deleted");
    assert_eq!(
        json_body(get(&client, &one).await).await,
        before,
        "nothing was changed or confirmed"
    );
}
