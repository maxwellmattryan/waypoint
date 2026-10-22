//! Stages and the timeline: the migration that adds them, the events every write records,
//! the stage routes, notes and rejection feedback.

use std::time::Duration;

use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use waypoint_api::claude::ClaudeRunner;

fn host() -> Header<'static> {
    Header::new("Host", "127.0.0.1")
}

async fn client(pool: PgPool) -> Client {
    // No application route calls the runner.
    let claude = ClaudeRunner::new("claude", Duration::from_secs(1));
    let rocket = waypoint_api::rocket(
        rocket::Config::figment(),
        pool,
        claude,
        waypoint_api::jobs::Config::default(),
        waypoint_api::google::Config::default(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
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

async fn get<'c>(client: &'c Client, path: &str) -> LocalResponse<'c> {
    client.get(path.to_string()).header(host()).dispatch().await
}

async fn json_body(response: LocalResponse<'_>) -> Value {
    response.into_json().await.expect("json body")
}

/// Creates an application and returns it.
async fn create(client: &Client, body: Value) -> Value {
    let response = send(client, Method::Post, "/applications", &body).await;
    assert_eq!(response.status(), Status::Created, "{body}");
    json_body(response).await
}

fn id(value: &Value) -> i64 {
    value["id"].as_i64().expect("id")
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

fn error(field: &str, message: &str) -> Vec<(String, String)> {
    vec![(field.to_string(), message.to_string())]
}

/// Moves an application's timestamps a minute into the past, so a later `now()` is
/// strictly greater whatever the clock's resolution.
async fn age(pool: &PgPool, id: i64) {
    sqlx::query(
        "UPDATE applications SET updated_at = now() - interval '1 minute', \
         status_changed_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("age row");
}

async fn application(client: &Client, id: i64) -> Value {
    let response = get(client, &format!("/applications/{id}")).await;
    assert_eq!(response.status(), Status::Ok);
    json_body(response).await
}

async fn events(client: &Client, id: i64) -> Vec<Value> {
    let response = get(client, &format!("/applications/{id}/events")).await;
    assert_eq!(response.status(), Status::Ok);
    json_body(response).await.as_array().expect("array").clone()
}

/// The timeline as `(kind, from, to, stage_name, body)`, newest first, with nulls as "".
fn summary(events: &[Value]) -> Vec<[String; 5]> {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    events
        .iter()
        .map(|e| {
            [
                text(&e["kind"]),
                text(&e["from_status"]),
                text(&e["to_status"]),
                text(&e["stage_name"]),
                text(&e["body"]),
            ]
        })
        .collect()
}

fn row(kind: &str, from: &str, to: &str, stage: &str, body: &str) -> [String; 5] {
    [kind, from, to, stage, body].map(str::to_string)
}

/// The stages as `(name, position, completed)`.
fn stage_rows(stages: &Value) -> Vec<(String, i64, bool)> {
    stages
        .as_array()
        .expect("array")
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["position"].as_i64().unwrap(),
                !s["completed_at"].is_null(),
            )
        })
        .collect()
}

fn stage_ids(stages: &Value) -> Vec<i64> {
    stages.as_array().expect("array").iter().map(id).collect()
}

async fn add_stage(client: &Client, app: i64, name: &str) -> Value {
    let response = send(
        client,
        Method::Post,
        &format!("/applications/{app}/stages"),
        &json!({"name": name}),
    )
    .await;
    assert_eq!(response.status(), Status::Created, "{name}");
    json_body(response).await
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM application_events")
        .fetch_one(pool)
        .await
        .expect("count events")
}

#[sqlx::test]
async fn schema_constraints_and_cascade(pool: PgPool) {
    let app: i64 = sqlx::query_scalar(
        "INSERT INTO applications (company, title) VALUES ('Acme', 'Engineer') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert application");

    let unknown =
        sqlx::query("INSERT INTO application_events (application_id, kind) VALUES ($1, 'edited')")
            .bind(app)
            .execute(&pool)
            .await;
    assert!(unknown.is_err(), "the kind CHECK refuses 'edited'");
    let bad_status = sqlx::query(
        "INSERT INTO application_events (application_id, kind, to_status) \
         VALUES ($1, 'created', 'archived')",
    )
    .bind(app)
    .execute(&pool)
    .await;
    assert!(bad_status.is_err(), "the status CHECK refuses 'archived'");
    let negative = sqlx::query(
        "INSERT INTO application_stages (application_id, name, position) VALUES ($1, 'x', -1)",
    )
    .bind(app)
    .execute(&pool)
    .await;
    assert!(negative.is_err(), "positions are never negative");

    sqlx::query(
        "INSERT INTO application_stages (application_id, name, position) VALUES ($1, 'Onsite', 0)",
    )
    .bind(app)
    .execute(&pool)
    .await
    .expect("insert stage");
    sqlx::query(
        "INSERT INTO application_events (application_id, kind, body) VALUES ($1, 'note', 'Hi')",
    )
    .bind(app)
    .execute(&pool)
    .await
    .expect("insert event");

    sqlx::query("DELETE FROM applications WHERE id = $1")
        .bind(app)
        .execute(&pool)
        .await
        .expect("delete application");
    let left: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM application_stages), (SELECT count(*) FROM application_events)",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(left, (0, 0), "stages and events go with their application");
}

#[sqlx::test(migrations = false)]
async fn backfill_adds_created_events(pool: PgPool) {
    // Apply every migration before this job's from a copy of the directory, add an
    // application, then apply the rest.
    let dir = tempfile::tempdir().expect("temp dir");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<_> = std::fs::read_dir(&source)
        .expect("migrations dir")
        .map(|entry| entry.expect("entry").path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 7);
    for file in &files[..5] {
        std::fs::copy(file, dir.path().join(file.file_name().unwrap())).expect("copy");
    }
    Migrator::new(dir.path())
        .await
        .expect("read migrations")
        .run(&pool)
        .await
        .expect("first five apply");

    let (app, created_at): (i64, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO applications (company, title, status, created_at) \
         VALUES ('Acme', 'Engineer', 'applied', now() - interval '1 day') \
         RETURNING id, created_at",
    )
    .fetch_one(&pool)
    .await
    .expect("insert application");

    waypoint_api::MIGRATOR
        .run(&pool)
        .await
        .expect("the rest apply");

    let rows: Vec<(i64, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT application_id, kind, to_status, created_at FROM application_events",
    )
    .fetch_all(&pool)
    .await
    .expect("events");
    assert_eq!(rows, vec![(app, "created".to_string(), None, created_at)]);
}

#[sqlx::test]
async fn create_and_status_changes_are_recorded(pool: PgPool) {
    let client = client(pool.clone()).await;
    let created = create(
        &client,
        json!({"company": "Acme", "title": "Engineer", "status": "applied"}),
    )
    .await;
    let app = id(&created);

    let timeline = events(&client, app).await;
    assert_eq!(
        summary(&timeline),
        vec![row("created", "", "applied", "", "")]
    );
    assert_eq!(timeline[0]["application_id"], app);
    assert_eq!(timeline[0]["created_at"], created["created_at"]);

    let path = format!("/applications/{app}/status");
    for status in ["interviewing", "interviewing", "offer"] {
        let response = send(&client, Method::Put, &path, &json!({"status": status})).await;
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(json_body(response).await["status"], status);
    }
    assert_eq!(
        summary(&events(&client, app).await),
        vec![
            row("status_changed", "interviewing", "offer", "", ""),
            row("status_changed", "applied", "interviewing", "", ""),
            row("created", "", "applied", "", ""),
        ],
        "newest first; setting the same status records nothing"
    );

    // Another application's events stay its own.
    let other = create(&client, json!({"company": "Globex", "title": "Engineer"})).await;
    assert_eq!(
        summary(&events(&client, id(&other)).await),
        vec![row("created", "", "sourced", "", "")]
    );

    let response = get(&client, "/applications/999999/events").await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn rejection_feedback_rules(pool: PgPool) {
    let client = client(pool.clone()).await;
    let app = id(&create(&client, json!({"company": "Acme", "title": "Engineer"})).await);
    let status = format!("/applications/{app}/status");
    let entries = format!("/applications/{app}/events");
    let before = application(&client, app).await;

    // Feedback needs `rejected`, both with the status and on its own.
    let response = send(
        &client,
        Method::Put,
        &status,
        &json!({"status": "applied", "feedback": "Nope"}),
    )
    .await;
    assert_eq!(response.status(), Status::UnprocessableEntity);
    assert_eq!(
        errors(&json_body(response).await),
        error("feedback", "Only when the status is rejected")
    );
    let response = send(
        &client,
        Method::Post,
        &entries,
        &json!({"kind": "rejection_feedback", "body": "Nope"}),
    )
    .await;
    assert_eq!(response.status(), Status::UnprocessableEntity);
    assert_eq!(
        errors(&json_body(response).await),
        error("kind", "Only for a rejected application")
    );
    let response = send(
        &client,
        Method::Put,
        &status,
        &json!({"status": "rejected", "feedback": "x".repeat(20_001)}),
    )
    .await;
    assert_eq!(response.status(), Status::UnprocessableEntity);
    assert_eq!(
        errors(&json_body(response).await),
        error("feedback", "At most 20000 characters")
    );
    assert_eq!(application(&client, app).await, before, "nothing changed");
    assert_eq!(events(&client, app).await.len(), 1, "only `created`");

    // Blank feedback with another status is no feedback.
    let response = send(
        &client,
        Method::Put,
        &status,
        &json!({"status": "applied", "feedback": "   "}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);

    let response = send(
        &client,
        Method::Put,
        &status,
        &json!({"status": "rejected", "feedback": "  Went with someone more senior.\n"}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(json_body(response).await["status"], "rejected");
    assert_eq!(
        summary(&events(&client, app).await),
        vec![
            row(
                "rejection_feedback",
                "",
                "",
                "",
                "Went with someone more senior."
            ),
            row("status_changed", "applied", "rejected", "", ""),
            row("status_changed", "sourced", "applied", "", ""),
            row("created", "", "sourced", "", ""),
        ]
    );

    // Already rejected: blank feedback is a complete no-op ...
    age(&pool, app).await;
    let aged = application(&client, app).await;
    let response = send(
        &client,
        Method::Put,
        &status,
        &json!({"status": "rejected", "feedback": ""}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(json_body(response).await, aged);
    assert_eq!(events(&client, app).await.len(), 4);

    // ... and real feedback writes only that event and moves only `updated_at`.
    let response = send(
        &client,
        Method::Put,
        &status,
        &json!({"status": "rejected", "feedback": "Reapply in a year"}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    let changed = json_body(response).await;
    assert_eq!(changed["status_changed_at"], aged["status_changed_at"]);
    assert!(changed["updated_at"].as_str() > aged["updated_at"].as_str());
    let timeline = events(&client, app).await;
    assert_eq!(timeline.len(), 5);
    assert_eq!(
        summary(&timeline[..1]),
        vec![row("rejection_feedback", "", "", "", "Reapply in a year")]
    );

    // Feedback can also be added on its own once rejected.
    let response = send(
        &client,
        Method::Post,
        &entries,
        &json!({"kind": "rejection_feedback", "body": " Call notes "}),
    )
    .await;
    assert_eq!(response.status(), Status::Created);
    let event = json_body(response).await;
    assert_eq!(
        summary(std::slice::from_ref(&event)),
        vec![row("rejection_feedback", "", "", "", "Call notes")]
    );
    assert_eq!(events(&client, app).await[0], event);
}

#[sqlx::test]
async fn notes_are_validated(pool: PgPool) {
    let client = client(pool.clone()).await;
    let app = id(&create(&client, json!({"company": "Acme", "title": "Engineer"})).await);
    let path = format!("/applications/{app}/events");

    for (body, want) in [
        (
            json!({"kind": "note", "body": "  \n "}),
            error("body", "Required"),
        ),
        (json!({"kind": "note"}), error("body", "Required")),
        (
            json!({"kind": "note", "body": "x".repeat(20_001)}),
            error("body", "At most 20000 characters"),
        ),
    ] {
        let response = send(&client, Method::Post, &path, &body).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{body}");
        assert_eq!(errors(&json_body(response).await), want, "{body}");
    }
    for body in [
        json!({"kind": "created", "body": "Hi"}),
        json!({"kind": "edited", "body": "Hi"}),
        json!({"body": "Hi"}),
    ] {
        let response = send(&client, Method::Post, &path, &body).await;
        assert_eq!(response.status(), Status::BadRequest, "{body}");
    }
    assert_eq!(events(&client, app).await.len(), 1, "nothing was written");

    age(&pool, app).await;
    let before = application(&client, app).await;
    let response = send(
        &client,
        Method::Post,
        &path,
        &json!({"kind": "note", "body": "  Line one\nLine two  "}),
    )
    .await;
    assert_eq!(response.status(), Status::Created);
    let note = json_body(response).await;
    assert_eq!(
        summary(std::slice::from_ref(&note)),
        vec![row("note", "", "", "", "Line one\nLine two")]
    );
    assert_eq!(note["application_id"], app);
    let after = application(&client, app).await;
    assert!(after["updated_at"].as_str() > before["updated_at"].as_str());
    assert_eq!(after["status_changed_at"], before["status_changed_at"]);

    let response = send(
        &client,
        Method::Post,
        &path,
        &json!({"kind": "note", "body": "é".repeat(20_000)}),
    )
    .await;
    assert_eq!(
        response.status(),
        Status::Created,
        "the limit counts characters"
    );

    let response = send(
        &client,
        Method::Post,
        "/applications/999999/events",
        &json!({"kind": "note", "body": "Hi"}),
    )
    .await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn stages_add_complete_reorder_remove(pool: PgPool) {
    let client = client(pool.clone()).await;
    let app = id(&create(&client, json!({"company": "Acme", "title": "Engineer"})).await);
    let list = format!("/applications/{app}/stages");

    let response = get(&client, &list).await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(json_body(response).await, json!([]));

    add_stage(&client, app, "Recruiter screen").await;
    add_stage(&client, app, "  Onsite ").await;
    let stages = add_stage(&client, app, "Offer call").await;
    assert_eq!(
        stage_rows(&stages),
        vec![
            ("Recruiter screen".to_string(), 0, false),
            ("Onsite".to_string(), 1, false),
            ("Offer call".to_string(), 2, false),
        ]
    );
    assert!(stages[0]["created_at"].is_string());
    assert_eq!(stages[0]["application_id"], app);
    assert_eq!(json_body(get(&client, &list).await).await, stages);
    let [screen, onsite, offer] = stage_ids(&stages)[..] else {
        panic!("three stages")
    };

    let one = |stage: i64| format!("/applications/{app}/stages/{stage}");
    for completed in [true, true] {
        let response = send(
            &client,
            Method::Put,
            &one(onsite),
            &json!({"completed": completed}),
        )
        .await;
        assert_eq!(response.status(), Status::Ok);
        let stages = json_body(response).await;
        assert_eq!(stage_rows(&stages)[1], ("Onsite".to_string(), 1, true));
    }
    let response = send(
        &client,
        Method::Put,
        &one(onsite),
        &json!({"completed": false}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    assert!(json_body(response).await[1]["completed_at"].is_null());
    let response = send(
        &client,
        Method::Put,
        &one(screen),
        &json!({"completed": true}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);

    // Reorder records nothing but moves `updated_at`.
    let events_before = events(&client, app).await.len();
    age(&pool, app).await;
    let before = application(&client, app).await;
    let response = send(
        &client,
        Method::Put,
        &format!("/applications/{app}/stage-order"),
        &json!({"stage_ids": [offer, screen, onsite]}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    let stages = json_body(response).await;
    assert_eq!(
        stage_rows(&stages),
        vec![
            ("Offer call".to_string(), 0, false),
            ("Recruiter screen".to_string(), 1, true),
            ("Onsite".to_string(), 2, false),
        ]
    );
    assert_eq!(stage_ids(&stages), vec![offer, screen, onsite]);
    assert_eq!(events(&client, app).await.len(), events_before);
    assert!(application(&client, app).await["updated_at"].as_str() > before["updated_at"].as_str());

    let response = client.delete(one(screen)).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        stage_rows(&json_body(response).await),
        vec![
            ("Offer call".to_string(), 0, false),
            ("Onsite".to_string(), 1, false)
        ],
        "later positions close the gap"
    );
    let response = client.delete(one(screen)).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NotFound, "already removed");

    // The removed stage still reads in the timeline.
    assert_eq!(
        summary(&events(&client, app).await),
        vec![
            row("stage_removed", "", "", "Recruiter screen", ""),
            row("stage_completed", "", "", "Recruiter screen", ""),
            row("stage_reopened", "", "", "Onsite", ""),
            row("stage_completed", "", "", "Onsite", ""),
            row("stage_added", "", "", "Offer call", ""),
            row("stage_added", "", "", "Onsite", ""),
            row("stage_added", "", "", "Recruiter screen", ""),
            row("created", "", "sourced", "", ""),
        ]
    );
}

#[sqlx::test]
async fn stage_validation_and_ownership(pool: PgPool) {
    let client = client(pool.clone()).await;
    let app = id(&create(&client, json!({"company": "Acme", "title": "Engineer"})).await);
    let other = id(&create(&client, json!({"company": "Globex", "title": "Engineer"})).await);
    let list = format!("/applications/{app}/stages");
    let order = format!("/applications/{app}/stage-order");

    for (body, want) in [
        (json!({"name": "   "}), error("name", "Required")),
        (json!({}), error("name", "Required")),
        (
            json!({"name": "x".repeat(201)}),
            error("name", "At most 200 characters"),
        ),
    ] {
        let response = send(&client, Method::Post, &list, &body).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{body}");
        assert_eq!(errors(&json_body(response).await), want, "{body}");
    }
    let response = send(&client, Method::Post, &list, &json!({"name": 5})).await;
    assert_eq!(response.status(), Status::BadRequest);
    add_stage(&client, app, &"é".repeat(200)).await;

    let stages = add_stage(&client, app, "Onsite").await;
    let [first, second] = stage_ids(&stages)[..] else {
        panic!("two stages")
    };
    let foreign = stage_ids(&add_stage(&client, other, "Screen").await)[0];

    // `stage_ids` must be exactly this application's stages, each once.
    for ids in [
        json!([first]),
        json!([first, second, second]),
        json!([first, first]),
        json!([first, foreign]),
        json!([first, second, foreign]),
        json!([]),
    ] {
        let response = send(&client, Method::Put, &order, &json!({"stage_ids": ids})).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{ids}");
        assert_eq!(
            errors(&json_body(response).await),
            error(
                "stage_ids",
                "Must list every stage of this application once"
            ),
            "{ids}"
        );
    }
    let response = send(&client, Method::Put, &order, &json!({"stage_ids": "1,2"})).await;
    assert_eq!(response.status(), Status::BadRequest);

    // A stage of another application is a 404 here, and stays as it was.
    let foreign_path = format!("/applications/{app}/stages/{foreign}");
    let response = send(
        &client,
        Method::Put,
        &foreign_path,
        &json!({"completed": true}),
    )
    .await;
    assert_eq!(response.status(), Status::NotFound);
    let response = client.delete(foreign_path).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NotFound);
    let response = get(&client, &format!("/applications/{other}/stages")).await;
    assert_eq!(
        stage_rows(&json_body(response).await),
        vec![("Screen".to_string(), 0, false)]
    );

    // Unknown application.
    let missing = "/applications/999999";
    assert_eq!(
        get(&client, &format!("{missing}/stages")).await.status(),
        Status::NotFound
    );
    for (method, path, body) in [
        (
            Method::Post,
            format!("{missing}/stages"),
            json!({"name": "Onsite"}),
        ),
        (
            Method::Put,
            format!("{missing}/stages/{first}"),
            json!({"completed": true}),
        ),
        (
            Method::Put,
            format!("{missing}/stage-order"),
            json!({"stage_ids": []}),
        ),
    ] {
        let response = send(&client, method, &path, &body).await;
        assert_eq!(response.status(), Status::NotFound, "{method} {path}");
    }
    let response = client
        .delete(format!("{missing}/stages/{first}"))
        .header(host())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::NotFound);

    // At most 50 stages.
    sqlx::query(
        "INSERT INTO application_stages (application_id, name, position) \
         SELECT $1, 'Stage ' || n, n FROM generate_series(2, 49) AS n",
    )
    .bind(app)
    .execute(&pool)
    .await
    .expect("fill stages");
    let events_before = event_count(&pool).await;
    let response = send(
        &client,
        Method::Post,
        &list,
        &json!({"name": "One too many"}),
    )
    .await;
    assert_eq!(response.status(), Status::UnprocessableEntity);
    assert_eq!(
        errors(&json_body(response).await),
        error("name", "At most 50 stages")
    );
    assert_eq!(
        event_count(&pool).await,
        events_before,
        "nothing was recorded"
    );
    let response = get(&client, &list).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 50);
}
