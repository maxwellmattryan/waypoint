//! The applications routes: create, read, list, update, status change and delete, their
//! validation, the guards on every mutating route and the `application.*` events.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::BufReader;
use waypoint_api::claude::ClaudeRunner;

use common::next_message;

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
        waypoint_api::capture::Config::default(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
}

async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM applications")
        .fetch_one(pool)
        .await
        .expect("count applications")
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

/// Creates an application and returns it.
async fn create(client: &Client, body: Value) -> Value {
    let response = send(client, Method::Post, "/applications", &body).await;
    assert_eq!(response.status(), Status::Created, "{body}");
    json_body(response).await
}

async fn get(client: &Client, id: i64) -> LocalResponse<'_> {
    client
        .get(format!("/applications/{id}"))
        .header(host())
        .dispatch()
        .await
}

/// Moves every timestamp of row `id` a minute into the past, so a later `now()` is
/// strictly greater whatever the clock's resolution.
async fn age(pool: &PgPool, id: i64) {
    sqlx::query(
        "UPDATE applications SET updated_at = now() - interval '1 minute', \
         status_changed_at = now() - interval '1 minute', \
         created_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("age row");
}

fn id(application: &Value) -> i64 {
    application["id"].as_i64().expect("id")
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

#[sqlx::test]
async fn status_check_rejects_unknown_value(pool: PgPool) {
    let inserted = sqlx::query(
        "INSERT INTO applications (company, title, status) VALUES ('Acme', 'Engineer', 'archived')",
    )
    .execute(&pool)
    .await;
    assert!(inserted.is_err(), "the CHECK constraint refuses 'archived'");

    let status: String = sqlx::query_scalar(
        "INSERT INTO applications (company, title) VALUES ('Acme', 'Engineer') RETURNING status",
    )
    .fetch_one(&pool)
    .await
    .expect("insert with defaults");
    assert_eq!(status, "sourced");

    let missing = sqlx::query("INSERT INTO applications (title) VALUES ('Engineer')")
        .execute(&pool)
        .await;
    assert!(missing.is_err(), "company is NOT NULL");
}

#[sqlx::test]
async fn create_then_get_round_trips(pool: PgPool) {
    let client = client(pool).await;
    let created = create(
        &client,
        json!({"company": "  Acme  ", "title": "Engineer\n", "location": "   ", "notes": ""}),
    )
    .await;

    assert_eq!(created["company"], "Acme");
    assert_eq!(created["title"], "Engineer");
    assert_eq!(created["status"], "sourced");
    for field in ["url", "location", "comp", "source", "notes"] {
        assert_eq!(created[field], Value::Null, "{field}");
    }
    for field in ["status_changed_at", "created_at", "updated_at"] {
        assert!(created[field].is_string(), "{field} is set");
    }

    let response = get(&client, id(&created)).await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(json_body(response).await, created);

    let applied = create(
        &client,
        json!({
            "company": "Globex",
            "title": "Staff Engineer",
            "url": "https://globex.example/jobs/7",
            "location": "Remote",
            "comp": "$180k–$210k + equity",
            "source": "Referral",
            "notes": "Line one\nLine two",
            "status": "applied",
        }),
    )
    .await;
    assert_eq!(applied["status"], "applied");
    assert_eq!(applied["url"], "https://globex.example/jobs/7");
    assert_eq!(applied["comp"], "$180k–$210k + equity");
    assert_eq!(applied["notes"], "Line one\nLine two");
    let response = get(&client, id(&applied)).await;
    assert_eq!(json_body(response).await, applied);
}

#[sqlx::test]
async fn invalid_input_is_rejected(pool: PgPool) {
    let client = client(pool.clone()).await;
    let long = |n: usize| "x".repeat(n);
    let unprocessable: Vec<(Value, Vec<(&str, &str)>)> = vec![
        (
            json!({}),
            vec![("company", "Required"), ("title", "Required")],
        ),
        (
            json!({"company": "   ", "title": "Engineer"}),
            vec![("company", "Required")],
        ),
        (
            json!({"company": "Acme", "title": "\t"}),
            vec![("title", "Required")],
        ),
        (
            json!({"company": "Acme", "title": "Engineer", "url": "javascript:alert(1)"}),
            vec![("url", "Must be an http or https URL")],
        ),
        (
            json!({"company": "Acme", "title": "Engineer", "url": "ftp://x"}),
            vec![("url", "Must be an http or https URL")],
        ),
        (
            json!({"company": "Acme", "title": "Engineer", "url": "example.com"}),
            vec![("url", "Must be an http or https URL")],
        ),
        (
            json!({
                "company": long(201),
                "title": long(201),
                "url": format!("https://a.example/{}", long(2048)),
                "location": long(201),
                "comp": long(201),
                "source": long(201),
                "notes": long(20_001),
            }),
            vec![
                ("company", "At most 200 characters"),
                ("title", "At most 200 characters"),
                ("url", "At most 2048 characters"),
                ("location", "At most 200 characters"),
                ("comp", "At most 200 characters"),
                ("source", "At most 200 characters"),
                ("notes", "At most 20000 characters"),
            ],
        ),
    ];
    for (body, want) in unprocessable {
        let response = send(&client, Method::Post, "/applications", &body).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{body}");
        let got = errors(&json_body(response).await);
        let want: Vec<(String, String)> = want
            .into_iter()
            .map(|(f, m)| (f.to_string(), m.to_string()))
            .collect();
        assert_eq!(got, want, "{body}");
    }

    // Exactly at the limits (counted in characters, not bytes) is fine.
    let at_limit = create(
        &client,
        json!({"company": "é".repeat(200), "title": "Engineer", "notes": long(20_000)}),
    )
    .await;
    sqlx::query("DELETE FROM applications WHERE id = $1")
        .bind(id(&at_limit))
        .execute(&pool)
        .await
        .unwrap();

    let bad_body = json!({"errors": [{
        "field": "body",
        "message": "The request body is not valid JSON for this route",
    }]});
    for raw in [
        "{not json",
        r#"{"company": 5, "title": "Engineer"}"#,
        r#"{"company": "Acme", "title": "Engineer", "status": "archived"}"#,
        "null",
    ] {
        let response = client
            .post("/applications")
            .header(host())
            .header(ContentType::JSON)
            .body(raw)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest, "{raw}");
        assert_eq!(json_body(response).await, bad_body, "{raw}");
    }

    // The same rules apply to an update and to a status change.
    let created = create(&client, json!({"company": "Acme", "title": "Engineer"})).await;
    let path = format!("/applications/{}", id(&created));
    let response = send(&client, Method::Put, &path, &json!({"company": ""})).await;
    assert_eq!(response.status(), Status::UnprocessableEntity);
    assert_eq!(
        errors(&json_body(response).await),
        vec![
            ("company".to_string(), "Required".to_string()),
            ("title".to_string(), "Required".to_string()),
        ]
    );
    let response = send(
        &client,
        Method::Put,
        &format!("{path}/status"),
        &json!({"status": "archived"}),
    )
    .await;
    assert_eq!(response.status(), Status::BadRequest);
    assert_eq!(json_body(response).await, bad_body);

    let response = get(&client, id(&created)).await;
    assert_eq!(json_body(response).await, created, "nothing changed");
    assert_eq!(count(&pool).await, 1, "only the valid create wrote a row");
}

#[sqlx::test]
async fn list_orders_by_last_update(pool: PgPool) {
    let client = client(pool.clone()).await;
    let list = || async {
        let response = client.get("/applications").header(host()).dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        json_body(response).await
    };
    assert_eq!(list().await, json!([]));

    let first = create(&client, json!({"company": "First", "title": "A"})).await;
    let second = create(&client, json!({"company": "Second", "title": "B"})).await;
    let third = create(&client, json!({"company": "Third", "title": "C"})).await;
    // Give every row the same `updated_at`, so the order falls back to `id` descending.
    sqlx::query("UPDATE applications SET updated_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    let ids = |list: Value| -> Vec<i64> { list.as_array().unwrap().iter().map(id).collect() };
    assert_eq!(ids(list().await), vec![id(&third), id(&second), id(&first)]);

    let response = send(
        &client,
        Method::Put,
        &format!("/applications/{}", id(&first)),
        &json!({"company": "First", "title": "A2"}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(ids(list().await), vec![id(&first), id(&third), id(&second)]);
}

#[sqlx::test]
async fn update_replaces_fields_but_not_status(pool: PgPool) {
    let client = client(pool.clone()).await;
    let created = create(
        &client,
        json!({
            "company": "Acme",
            "title": "Engineer",
            "url": "https://acme.example/1",
            "location": "Berlin",
            "comp": "€90k",
            "source": "LinkedIn",
            "notes": "Old notes",
            "status": "interviewing",
        }),
    )
    .await;
    age(&pool, id(&created)).await;
    let before = json_body(get(&client, id(&created)).await).await;

    let response = send(
        &client,
        Method::Put,
        &format!("/applications/{}", id(&created)),
        &json!({"company": " Acme GmbH ", "title": "Senior Engineer", "location": "Remote", "status": "offer"}),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    let updated = json_body(response).await;

    assert_eq!(updated["company"], "Acme GmbH");
    assert_eq!(updated["title"], "Senior Engineer");
    assert_eq!(updated["location"], "Remote");
    for field in ["url", "comp", "source", "notes"] {
        assert_eq!(updated[field], Value::Null, "omitted {field} is cleared");
    }
    assert_eq!(updated["status"], "interviewing", "status is untouched");
    assert_eq!(updated["status_changed_at"], before["status_changed_at"]);
    assert_eq!(updated["created_at"], before["created_at"]);
    assert!(
        updated["updated_at"].as_str() > before["updated_at"].as_str(),
        "updated_at moves forward"
    );
    assert_eq!(json_body(get(&client, id(&created)).await).await, updated);

    let response = send(
        &client,
        Method::Put,
        "/applications/999999",
        &json!({"company": "Acme", "title": "Engineer"}),
    )
    .await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn status_change_sets_timestamps_once(pool: PgPool) {
    let client = client(pool.clone()).await;
    let created = create(&client, json!({"company": "Acme", "title": "Engineer"})).await;
    age(&pool, id(&created)).await;
    let before = json_body(get(&client, id(&created)).await).await;
    let path = format!("/applications/{}/status", id(&created));

    let response = send(&client, Method::Put, &path, &json!({"status": "applied"})).await;
    assert_eq!(response.status(), Status::Ok);
    let changed = json_body(response).await;
    assert_eq!(changed["status"], "applied");
    assert!(changed["status_changed_at"].as_str() > before["status_changed_at"].as_str());
    assert!(changed["updated_at"].as_str() > before["updated_at"].as_str());
    assert_eq!(changed["created_at"], before["created_at"]);

    age(&pool, id(&created)).await;
    let aged = json_body(get(&client, id(&created)).await).await;
    let response = send(&client, Method::Put, &path, &json!({"status": "applied"})).await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        json_body(response).await,
        aged,
        "the same status changes nothing"
    );

    let response = send(
        &client,
        Method::Put,
        "/applications/999999/status",
        &json!({"status": "offer"}),
    )
    .await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn delete_removes_the_row(pool: PgPool) {
    let client = client(pool.clone()).await;
    let created = create(&client, json!({"company": "Acme", "title": "Engineer"})).await;
    let path = format!("/applications/{}", id(&created));

    let response = client.delete(path.clone()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NoContent);
    assert_eq!(get(&client, id(&created)).await.status(), Status::NotFound);
    assert_eq!(count(&pool).await, 0);

    let response = client.delete(path).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::NotFound);
}

#[sqlx::test]
async fn guards_refuse_foreign_requests(pool: PgPool) {
    let client = client(pool.clone()).await;
    let created = create(&client, json!({"company": "Acme", "title": "Engineer"})).await;
    let one = format!("/applications/{}", id(&created));
    let status = format!("{one}/status");
    let stages = format!("{one}/stages");
    let response = send(&client, Method::Post, &stages, &json!({"name": "Onsite"})).await;
    assert_eq!(response.status(), Status::Created);
    let stages_before = json_body(response).await;
    let stage = format!("{stages}/{}", id(&stages_before[0]));
    let order = format!("{one}/stage-order");
    let entries = format!("{one}/events");
    let events_before =
        json_body(client.get(entries.clone()).header(host()).dispatch().await).await;
    let created = json_body(get(&client, id(&created)).await).await;
    let valid_create = json!({"company": "Evil", "title": "Engineer"}).to_string();
    let valid_update = json!({"company": "Evil", "title": "Engineer"}).to_string();
    let valid_status = json!({"status": "rejected", "feedback": "Evil"}).to_string();
    let valid_stage = json!({"name": "Evil"}).to_string();
    let valid_completed = json!({"completed": true}).to_string();
    let valid_order = json!({"stage_ids": [id(&stages_before[0])]}).to_string();
    let valid_entry = json!({"kind": "note", "body": "Evil"}).to_string();

    let mutating: [(Method, &str, &str); 9] = [
        (Method::Post, "/applications", &valid_create),
        (Method::Put, &one, &valid_update),
        (Method::Put, &status, &valid_status),
        (Method::Delete, &one, ""),
        (Method::Post, &stages, &valid_stage),
        (Method::Put, &stage, &valid_completed),
        (Method::Put, &order, &valid_order),
        (Method::Delete, &stage, ""),
        (Method::Post, &entries, &valid_entry),
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
                .body("company=Evil&title=Engineer")
                .dispatch()
                .await;
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "{form} {method} {path}"
            );
        }
    }

    // What any web page can auto-submit: a form POST Rocket turns into DELETE or PUT.
    for (path, override_) in [
        (&one, "delete"),
        (&one, "put"),
        (&status, "put"),
        (&stage, "put"),
        (&stage, "delete"),
        (&order, "put"),
    ] {
        let response = client
            .post(path.clone())
            .header(host())
            .header(ContentType::Form)
            .body(format!(
                "_method={override_}&status=rejected&completed=true"
            ))
            .dispatch()
            .await;
        assert_eq!(
            response.status(),
            Status::Forbidden,
            "_method={override_} {path}"
        );
    }

    // A body that isn't declared JSON never reaches the handler.
    for (method, path, body) in [
        (Method::Post, "/applications", &valid_create),
        (Method::Put, &one, &valid_update),
        (Method::Put, &status, &valid_status),
        (Method::Post, &stages, &valid_stage),
        (Method::Put, &stage, &valid_completed),
        (Method::Put, &order, &valid_order),
        (Method::Post, &entries, &valid_entry),
    ] {
        let response = client
            .req(method, path)
            .header(host())
            .header(ContentType::Plain)
            .body(body.as_str())
            .dispatch()
            .await;
        assert!(
            response.status().class().is_client_error(),
            "text/plain {method} {path}: {}",
            response.status()
        );
    }

    let evil = Header::new("Host", "evil.example");
    let all: [(Method, &str, &str); 13] = [
        (Method::Get, "/applications", ""),
        (Method::Post, "/applications", &valid_create),
        (Method::Get, &one, ""),
        (Method::Put, &one, &valid_update),
        (Method::Put, &status, &valid_status),
        (Method::Delete, &one, ""),
        (Method::Get, &stages, ""),
        (Method::Post, &stages, &valid_stage),
        (Method::Put, &stage, &valid_completed),
        (Method::Put, &order, &valid_order),
        (Method::Delete, &stage, ""),
        (Method::Get, &entries, ""),
        (Method::Post, &entries, &valid_entry),
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
        json_body(get(&client, id(&created)).await).await,
        created,
        "nothing was changed"
    );
    let response = client.get(stages).header(host()).dispatch().await;
    assert_eq!(
        json_body(response).await,
        stages_before,
        "no stage was added, changed, reordered or removed"
    );
    let response = client.get(entries).header(host()).dispatch().await;
    assert_eq!(
        json_body(response).await,
        events_before,
        "nothing was recorded"
    );
}

/// Opens `GET /events` and reads past its `app.connected`.
async fn open_events(client: &Client) -> BufReader<LocalResponse<'_>> {
    let stream = client.get("/events").header(host()).dispatch().await;
    assert_eq!(stream.status(), Status::Ok);
    let mut stream = BufReader::new(stream);
    let (event, _) = next_message(&mut stream).await;
    assert_eq!(event, "app.connected");
    stream
}

/// The next SSE message, checking that its `event:` field equals its `type`.
async fn next_event(stream: &mut BufReader<LocalResponse<'_>>) -> (String, Value) {
    let (event, data) = next_message(stream).await;
    assert_eq!(data["type"], event.as_str(), "the SSE event is the type");
    (event, data)
}

#[sqlx::test]
async fn mutations_publish_application_events(pool: PgPool) {
    let client = client(pool).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = open_events(&client).await;

        let created = create(&client, json!({"company": "Acme", "title": "Engineer"})).await;
        let (event, data) = next_event(&mut stream).await;
        assert_eq!(event, "application.created");
        assert_eq!(data["application"], created);

        let path = format!("/applications/{}", id(&created));
        let response = send(
            &client,
            Method::Put,
            &path,
            &json!({"company": "Acme", "title": "Senior Engineer"}),
        )
        .await;
        assert_eq!(response.status(), Status::Ok);
        let updated = json_body(response).await;
        let (event, data) = next_event(&mut stream).await;
        assert_eq!(event, "application.updated");
        assert_eq!(data["application"], updated);

        let response = send(
            &client,
            Method::Put,
            &format!("{path}/status"),
            &json!({"status": "applied"}),
        )
        .await;
        assert_eq!(response.status(), Status::Ok);
        let moved = json_body(response).await;
        let (event, data) = next_event(&mut stream).await;
        assert_eq!(event, "application.updated");
        assert_eq!(data["application"], moved);
        assert_eq!(data["application"]["status"], "applied");

        let response = send(
            &client,
            Method::Put,
            &format!("{path}/resume"),
            &json!({"resume_version_id": null}),
        )
        .await;
        assert_eq!(response.status(), Status::Ok);
        let linked = json_body(response).await;
        let (event, data) = next_event(&mut stream).await;
        assert_eq!(event, "application.updated");
        assert_eq!(data["application"], linked);

        let response = client.delete(path).header(host()).dispatch().await;
        assert_eq!(response.status(), Status::NoContent);
        let (event, data) = next_event(&mut stream).await;
        assert_eq!(event, "application.deleted");
        assert_eq!(
            data,
            json!({"type": "application.deleted", "id": id(&created)})
        );

        // Each mutation published exactly one event: the next one is this create's.
        let other = create(&client, json!({"company": "Globex", "title": "Staff"})).await;
        let (event, data) = next_event(&mut stream).await;
        assert_eq!(event, "application.created");
        assert_eq!(data["application"], other);
    })
    .await
    .expect("every event arrives within 10s");
}

#[sqlx::test]
async fn failed_mutations_publish_nothing(pool: PgPool) {
    let client = client(pool).await;
    let existing = create(&client, json!({"company": "Acme", "title": "Engineer"})).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = open_events(&client).await;

        let response = send(
            &client,
            Method::Put,
            "/applications/999999/status",
            &json!({"status": "offer"}),
        )
        .await;
        assert_eq!(response.status(), Status::NotFound);

        let response = send(&client, Method::Post, "/applications", &json!({})).await;
        assert_eq!(response.status(), Status::UnprocessableEntity);

        let response = client
            .delete(format!("/applications/{}", id(&existing)))
            .header(host())
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);

        let created = create(&client, json!({"company": "Globex", "title": "Staff"})).await;
        let (event, data) = next_event(&mut stream).await;
        assert_eq!(event, "application.created");
        assert_eq!(data["application"], created);
    })
    .await
    .expect("the create's event arrives within 10s");
}
