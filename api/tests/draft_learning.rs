//! The memory loop end to end through the API: a `tidy_application` draft edited on
//! approval becomes a learning signal, the memory writer proposes a memory from it, the
//! user confirms it, and the next draft for the same application carries it, with the run
//! inspector naming the memory that changed it. The writer plays `fake-claude.sh`'s
//! `memory_writer` mode; `tidy_application` calls no model.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;

const TIMEOUT: Duration = Duration::from_secs(20);

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The API with the production runtime and a runner playing `mode`. Keep the `Fake` alive
/// as long as the client.
async fn app(
    pool: &PgPool,
    mode: &str,
    config: waypoint_api::jobs::Config,
) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(30));
    let rocket = common::app(pool.clone(), fake.runner.clone(), config);
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// `(status, JSON body or null)`.
async fn post(client: &Client, path: &str, body: Option<Value>) -> (Status, Value) {
    let mut request = client.post(path.to_owned()).header(host());
    if let Some(body) = body {
        request = request.header(ContentType::JSON).body(body.to_string());
    }
    let response = request.dispatch().await;
    let status = response.status();
    let body = response
        .into_string()
        .await
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    (status, body)
}

async fn get(client: &Client, path: &str) -> Value {
    let response = client.get(path.to_owned()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Ok, "GET {path}");
    response.into_json().await.expect("a JSON body")
}

/// Waits until run `id` is `status`, and returns its output.
async fn until(pool: &PgPool, id: i64, status: &str) -> Value {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let (current, output): (String, Option<Value>) =
            sqlx::query_as("SELECT status, output FROM agent_runs WHERE id = $1")
                .bind(id)
                .fetch_one(pool)
                .await
                .expect("read the run");
        if current == status {
            return output.unwrap_or(Value::Null);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "run {id} is {current}, never {status}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Starts `tidy_application` with `body`, waits for it to park, and returns
/// `(run id, its pending action)`.
async fn parked_tidy(
    pool: &PgPool,
    client: &Client,
    application: i64,
    body: &Value,
) -> (i64, Value) {
    let (status, run) = post(
        client,
        &format!("/applications/{application}/tidy"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, Status::Created, "{run}");
    let run = run["id"].as_i64().expect("run id");
    until(pool, run, "awaiting_approval").await;
    let pending = get(client, "/pending-actions").await;
    let action = pending
        .as_array()
        .expect("an array")
        .iter()
        .find(|action| action["run_id"] == json!(run))
        .unwrap_or_else(|| panic!("no pending action for run {run}: {pending}"))
        .clone();
    assert_eq!(action["tool"], json!("draft_follow_up"), "{action}");
    (run, action)
}

#[sqlx::test]
async fn confirmed_memory_changes_the_next_draft(pool: PgPool) {
    let (client, _fake) = app(&pool, "memory_writer", common::fast_config()).await;
    let (status, application) = post(
        &client,
        "/applications",
        Some(json!({"company": "Acme", "title": "Platform Engineer"})),
    )
    .await;
    assert_eq!(status, Status::Created, "{application}");
    let application = application["id"].as_i64().expect("application id");
    let body = json!({
        "note": "Called the recruiter",
        "message": "Following up on the remote role",
        "confidence": 0.9,
    });

    // The first draft: no memories yet, so the message is the input's.
    let (first, action) = parked_tidy(&pool, &client, application, &body).await;
    let first_message = action["input"]["message"].clone();
    assert_eq!(first_message, json!("Following up on the remote role"));
    assert_eq!(
        get(&client, &format!("/runs/{first}")).await["memories"],
        json!([])
    );

    // Approving an edited draft records one approval signal.
    let pending = action["id"].as_i64().expect("pending action id");
    let (status, approved) = post(
        &client,
        &format!("/pending-actions/{pending}/approve"),
        Some(json!({"input": {
            "application_id": application,
            "message": "Following up on the remote role. I'm only looking at remote roles.",
        }})),
    )
    .await;
    assert_eq!(status, Status::Ok, "{approved}");
    until(&pool, first, "succeeded").await;
    let signals = get(&client, "/memory-signals").await;
    let signals = signals.as_array().expect("an array");
    assert_eq!(signals.len(), 1, "{signals:?}");
    assert_eq!(
        (&signals[0]["kind"], &signals[0]["id"]),
        (&json!("approval"), &json!(1))
    );

    // The writer run the approval scheduled starts now and proposes one memory.
    let (status, learned) = post(&client, "/memories/learn", None).await;
    assert_eq!(status, Status::Accepted, "{learned}");
    assert_eq!(learned["reused"], json!(true), "{learned}");
    let writer = learned["run_id"].as_i64().expect("writer run id");
    until(&pool, writer, "succeeded").await;
    let all = get(&client, "/memories").await;
    assert_eq!(all.as_array().map(Vec::len), Some(1), "{all}");
    let proposed = get(&client, "/memories?status=proposed").await;
    let proposed = proposed.as_array().expect("an array");
    assert_eq!(proposed.len(), 1, "{proposed:?}");
    assert_eq!(
        (&proposed[0]["content"], &proposed[0]["kind"]),
        (&json!("Remote only"), &json!("preference"))
    );
    let memory = proposed[0]["id"].as_i64().expect("memory id");

    // The user confirms it.
    let (status, confirmed) = post(&client, &format!("/memories/{memory}/confirm"), None).await;
    assert_eq!(status, Status::Ok, "{confirmed}");
    assert_eq!(confirmed["status"], json!("confirmed"), "{confirmed}");

    // The same request now drafts with the confirmed preference, and the run says why.
    let (second, action) = parked_tidy(&pool, &client, application, &body).await;
    let second_message = action["input"]["message"].clone();
    assert_ne!(second_message, first_message);
    assert_eq!(
        second_message,
        json!("Following up on the remote role\n\nRemote only")
    );
    let memories = get(&client, &format!("/runs/{second}")).await["memories"].clone();
    let memories = memories.as_array().expect("an array");
    assert_eq!(memories.len(), 1, "{memories:?}");
    assert_eq!(
        (
            &memories[0]["memory_id"],
            &memories[0]["step"],
            &memories[0]["kind"],
            &memories[0]["content"],
        ),
        (
            &json!(memory),
            &json!("draft"),
            &json!("preference"),
            &json!("Remote only"),
        )
    );
    assert_eq!(
        get(&client, &format!("/runs/{first}")).await["memories"],
        json!([])
    );
}
