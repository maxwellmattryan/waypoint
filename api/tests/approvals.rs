//! The guardrails end to end through the API: the `tidy_application` sample workflow parks
//! at the approval gate, approve and reject decide it, the action log undoes its note, and
//! a low-confidence note goes to the review queue. The last tests drive the registry's gate
//! directly to show an approved action runs at most once. No model is called.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::Notify;
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::events::{Event, EventBus, Subscriber};
use waypoint_api::runs::RunStatus;
use waypoint_api::tools::{Permission, Registry, Tool, ToolCallError, ToolContext, ToolError};

const TIMEOUT: Duration = Duration::from_secs(10);

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The API with the production runtime, and a subscriber to its bus.
async fn app(pool: &PgPool) -> (Client, Subscriber) {
    let rocket = common::app(
        pool.clone(),
        ClaudeRunner::new("claude", Duration::from_secs(1)),
        common::fast_config(),
    );
    let client = Client::tracked(rocket).await.expect("valid rocket");
    let subscriber = client
        .rocket()
        .state::<EventBus>()
        .expect("the bus")
        .subscribe();
    (client, subscriber)
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

/// Every learning signal, newest first.
async fn signals(client: &Client) -> Vec<Value> {
    get(client, "/memory-signals")
        .await
        .as_array()
        .expect("an array")
        .clone()
}

async fn signal_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM memory_signals")
        .fetch_one(pool)
        .await
        .expect("count signals")
}

async fn create_application(client: &Client) -> i64 {
    let (status, body) = post(
        client,
        "/applications",
        Some(json!({"company": "Acme", "title": "Engineer"})),
    )
    .await;
    assert_eq!(status, Status::Created);
    body["id"].as_i64().expect("id")
}

/// Starts `tidy_application` for `application` and returns the queued run's id.
async fn start(client: &Client, application: i64, confidence: f64) -> i64 {
    let (status, run) = post(
        client,
        &format!("/applications/{application}/tidy"),
        Some(json!({"note": "Ask about the team", "message": "Thanks", "confidence": confidence})),
    )
    .await;
    assert_eq!(status, Status::Created, "{run}");
    assert_eq!(
        (run["workflow"].as_str(), run["status"].as_str()),
        (Some("tidy_application"), Some("queued"))
    );
    run["id"].as_i64().expect("id")
}

/// Waits for a `run.updated` with `status` for run `id`.
async fn until(subscriber: &mut Subscriber, id: i64, status: RunStatus) {
    let events = common::events(
        subscriber,
        TIMEOUT,
        |event| matches!(event, Event::RunUpdated { run } if run.id == id && run.status == status),
    )
    .await;
    assert!(
        matches!(events.last(), Some(Event::RunUpdated { run }) if run.id == id && run.status == status),
        "run {id} never became {status:?}: {events:#?}"
    );
}

/// Starts a run and waits for it to park at `draft`; returns `(run id, pending action id)`.
async fn parked(
    client: &Client,
    subscriber: &mut Subscriber,
    application: i64,
    confidence: f64,
) -> (i64, i64) {
    let run = start(client, application, confidence).await;
    until(subscriber, run, RunStatus::AwaitingApproval).await;
    let pending = get(client, "/pending-actions").await;
    let action = pending
        .as_array()
        .expect("an array")
        .iter()
        .find(|action| action["run_id"] == json!(run))
        .expect("the run's pending action");
    (run, action["id"].as_i64().expect("id"))
}

/// `(name, status, attempts)` of a run's steps.
fn step_states(run: &Value) -> Vec<(String, String, i64)> {
    run["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|step| {
            (
                step["name"].as_str().unwrap_or_default().to_owned(),
                step["status"].as_str().unwrap_or_default().to_owned(),
                step["attempts"].as_i64().unwrap_or_default(),
            )
        })
        .collect()
}

/// `(status, count)` of a run's `draft_follow_up` tool calls, by status.
async fn draft_calls(pool: &PgPool, run: i64) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT t.status, count(*) FROM tool_calls AS t JOIN run_steps AS s \
         ON s.id = t.run_step_id WHERE s.run_id = $1 AND t.tool = 'draft_follow_up' \
         GROUP BY t.status ORDER BY t.status",
    )
    .bind(run)
    .fetch_all(pool)
    .await
    .expect("count draft calls")
}

async fn notes(client: &Client, application: i64) -> Vec<Value> {
    get(client, &format!("/applications/{application}/events"))
        .await
        .as_array()
        .expect("an array")
        .iter()
        .filter(|event| event["kind"] == "note")
        .cloned()
        .collect()
}

#[sqlx::test]
async fn sample_workflow_parks_approves_resumes_and_undoes(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, action) = parked(&client, &mut subscriber, application, 0.9).await;

    // Parked at `draft`: the handler hasn't run, and its call waits for approval.
    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["status"], "awaiting_approval");
    assert_eq!(
        step_states(&detail),
        [
            ("inspect".into(), "succeeded".into(), 1),
            ("note".into(), "succeeded".into(), 1),
            ("draft".into(), "awaiting_approval".into(), 1),
        ]
    );
    let pending = get(&client, "/pending-actions").await;
    assert_eq!(pending.as_array().map(Vec::len), Some(1), "{pending}");
    assert_eq!(pending[0]["tool"], "draft_follow_up");
    assert_eq!(pending[0]["status"], "pending");
    assert_eq!(
        pending[0]["input"],
        json!({"application_id": application, "message": "Thanks"})
    );
    let gate_call = &detail["steps"][2]["tool_calls"][0];
    assert_eq!(gate_call["id"], pending[0]["tool_call_id"]);
    assert_eq!(gate_call["status"], "awaiting_approval");
    assert_eq!(
        draft_calls(&pool, run).await,
        [("awaiting_approval".into(), 1)]
    );

    // Approving requeues the run, which resumes at `draft` and runs the handler once.
    let (status, approved) = post(
        &client,
        &format!("/pending-actions/{action}/approve"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Ok, "{approved}");
    assert_eq!(approved["status"], "approved");
    assert_eq!(approved["approved_input"], approved["input"]);
    let input = json!({"application_id": application, "message": "Thanks"});
    let recorded = signals(&client).await;
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    let signal = &recorded[0];
    assert_eq!(
        (
            &signal["kind"],
            &signal["run_id"],
            &signal["pending_action_id"],
            &signal["review_item_id"],
            &signal["subject"],
            &signal["reason"],
        ),
        (
            &json!("approval"),
            &json!(run),
            &json!(action),
            &Value::Null,
            &json!("draft_follow_up"),
            &Value::Null,
        )
    );
    assert_eq!(signal["drafted"], input);
    assert_eq!(signal["approved"], input);
    assert_eq!(signal["changes"], json!([]));
    until(&mut subscriber, run, RunStatus::Succeeded).await;

    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["output"], json!({"draft": "Thanks (to Acme)"}));
    assert_eq!(
        step_states(&detail),
        [
            ("inspect".into(), "succeeded".into(), 1),
            ("note".into(), "succeeded".into(), 1),
            ("draft".into(), "succeeded".into(), 2),
        ]
    );
    assert_eq!(
        draft_calls(&pool, run).await,
        [("awaiting_approval".into(), 1), ("succeeded".into(), 1)]
    );
    let actions: Vec<(String, Option<Value>, Option<i64>)> = sqlx::query_as(
        "SELECT status, output, executed_tool_call_id FROM pending_actions WHERE run_id = $1",
    )
    .bind(run)
    .fetch_all(&pool)
    .await
    .expect("read pending actions");
    assert_eq!(actions.len(), 1, "still one pending action");
    assert_eq!(actions[0].0, "executed");
    assert_eq!(actions[0].1, Some(json!({"draft": "Thanks (to Acme)"})));
    assert!(actions[0].2.is_some());
    assert!(get(&client, "/pending-actions").await == json!([]));
    let (status, _) = post(
        &client,
        &format!("/pending-actions/{action}/approve"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Conflict, "a second approve");
    assert_eq!(signal_count(&pool).await, 1, "no second signal");

    // `add_note` is in the action log, and undo deletes the note once.
    let note = notes(&client, application).await;
    assert_eq!(note.len(), 1);
    assert_eq!(note[0]["body"], "Ask about the team");
    let logged = get(&client, &format!("/actions?run_id={run}")).await;
    assert_eq!(logged.as_array().map(Vec::len), Some(1), "{logged}");
    assert_eq!(logged[0]["tool"], "add_note");
    assert_eq!(
        logged[0]["undo_data"],
        json!({"event_id": note[0]["id"], "application_id": application})
    );
    assert_eq!(logged[0]["undone_at"], Value::Null);
    let id = logged[0]["id"].as_i64().expect("id");

    let (status, undone) = post(&client, &format!("/actions/{id}/undo"), None).await;
    assert_eq!(status, Status::Ok, "{undone}");
    assert!(undone["undone_at"].is_string(), "{undone}");
    assert!(notes(&client, application).await.is_empty());

    let (status, _) = post(&client, &format!("/actions/{id}/undo"), None).await;
    assert_eq!(status, Status::Conflict, "a second undo");
    let after = get(&client, &format!("/actions?run_id={run}")).await;
    assert_eq!(after[0], undone, "the row is unchanged");
}

#[sqlx::test]
async fn edit_then_approve_uses_the_edited_input(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, action) = parked(&client, &mut subscriber, application, 1.0).await;
    let path = format!("/pending-actions/{action}/approve");

    let (status, body) = post(
        &client,
        &path,
        Some(json!({"input": {"application_id": application, "message": 5}})),
    )
    .await;
    assert_eq!(status, Status::UnprocessableEntity);
    assert_eq!(
        body,
        json!({"errors": [{"field": "input", "message": "$.message: expected string"}]})
    );
    let pending = get(&client, "/pending-actions").await;
    assert_eq!(pending[0]["id"], json!(action));
    assert_eq!(pending[0]["status"], "pending");
    assert_eq!(
        signal_count(&pool).await,
        0,
        "a refused edit stores no signal"
    );

    let edited = json!({"application_id": application, "message": "edited"});
    let (status, approved) = post(&client, &path, Some(json!({"input": edited}))).await;
    assert_eq!(status, Status::Ok, "{approved}");
    assert_eq!(approved["approved_input"], edited);
    let recorded = signals(&client).await;
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0]["kind"], "approval");
    assert_eq!(
        recorded[0]["drafted"],
        json!({"application_id": application, "message": "Thanks"})
    );
    assert_eq!(recorded[0]["approved"], edited);
    assert_eq!(
        recorded[0]["changes"],
        json!([{"op": "replace", "path": "/message", "before": "Thanks", "after": "edited"}])
    );
    until(&mut subscriber, run, RunStatus::Succeeded).await;
    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["output"], json!({"draft": "edited (to Acme)"}));
}

async fn set_kill_switch(client: &Client, paused: bool) {
    let response = client
        .put("/kill-switch")
        .header(host())
        .header(ContentType::JSON)
        .body(json!({"paused": paused}).to_string())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok, "PUT /kill-switch {paused}");
}

#[sqlx::test]
async fn approve_while_paused_waits_for_unpause(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, action) = parked(&client, &mut subscriber, application, 1.0).await;

    // The kill switch leaves a parked run parked.
    set_kill_switch(&client, true).await;
    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["status"], "awaiting_approval");

    // Approving stores the run `paused` with its job, and the parked step doesn't run.
    let (status, approved) = post(
        &client,
        &format!("/pending-actions/{action}/approve"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Ok, "{approved}");
    until(&mut subscriber, run, RunStatus::Paused).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["status"], "paused");
    assert_eq!(detail["steps"][2]["status"], "awaiting_approval");
    assert_eq!(
        draft_calls(&pool, run).await,
        [("awaiting_approval".into(), 1)],
        "the handler hasn't run"
    );
    let job: String = sqlx::query_scalar(
        "SELECT j.status FROM jobs AS j JOIN agent_runs AS r ON r.job_id = j.id WHERE r.id = $1",
    )
    .bind(run)
    .fetch_one(&pool)
    .await
    .expect("the run's job");
    assert_eq!(job, "queued");

    // Unpausing runs the approved action.
    set_kill_switch(&client, false).await;
    until(&mut subscriber, run, RunStatus::Succeeded).await;
    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["output"], json!({"draft": "Thanks (to Acme)"}));
    assert_eq!(
        draft_calls(&pool, run).await,
        [("awaiting_approval".into(), 1), ("succeeded".into(), 1)]
    );
}

#[sqlx::test]
async fn reject_ends_the_run(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, action) = parked(&client, &mut subscriber, application, 1.0).await;

    let (status, rejected) =
        post(&client, &format!("/pending-actions/{action}/reject"), None).await;
    assert_eq!(status, Status::Ok, "{rejected}");
    assert_eq!(rejected["status"], "rejected");
    assert!(rejected["decided_at"].is_string());
    until(&mut subscriber, run, RunStatus::Rejected).await;

    let error = format!("the user rejected pending action {action}");
    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["status"], "rejected");
    assert_eq!(detail["error"], json!(error));
    assert!(detail["finished_at"].is_string());
    let draft = &detail["steps"][2];
    assert_eq!(
        (&draft["name"], &draft["status"], &draft["error"]),
        (&json!("draft"), &json!("failed"), &json!(error))
    );
    assert!(draft["finished_at"].is_string());
    assert_eq!(
        draft_calls(&pool, run).await,
        [("awaiting_approval".into(), 1)],
        "the handler never ran"
    );

    let recorded = signals(&client).await;
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    let signal = &recorded[0];
    assert_eq!(
        (
            &signal["kind"],
            &signal["run_id"],
            &signal["pending_action_id"],
            &signal["subject"],
        ),
        (
            &json!("rejection"),
            &json!(run),
            &json!(action),
            &json!("draft_follow_up"),
        )
    );
    assert_eq!(
        signal["drafted"],
        json!({"application_id": application, "message": "Thanks"})
    );
    for field in ["approved", "changes", "reason", "review_item_id"] {
        assert_eq!(signal[field], Value::Null, "{field}");
    }

    for decision in ["approve", "reject"] {
        let body = (decision == "approve").then(|| json!({}));
        let (status, _) = post(
            &client,
            &format!("/pending-actions/{action}/{decision}"),
            body,
        )
        .await;
        assert_eq!(status, Status::Conflict, "{decision} after reject");
    }
    assert_eq!(signal_count(&pool).await, 1, "no second signal");
}

#[sqlx::test]
async fn low_confidence_result_goes_to_review(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, _) = parked(&client, &mut subscriber, application, 0.3).await;

    let items: Vec<(i64, String, String, f64, Value)> = sqlx::query_as(
        "SELECT id, kind, status, confidence, payload FROM review_items WHERE run_id = $1",
    )
    .bind(run)
    .fetch_all(&pool)
    .await
    .expect("read review items");
    assert_eq!(items.len(), 1);
    let (id, kind, status, confidence, payload) = &items[0];
    assert_eq!((kind.as_str(), status.as_str()), ("tidy.note", "pending"));
    assert!((confidence - 0.3).abs() < 1e-12);
    assert_eq!(
        payload,
        &json!({"application_id": application, "note": "Ask about the team"})
    );

    assert!(notes(&client, application).await.is_empty(), "no note");
    assert_eq!(
        get(&client, &format!("/actions?run_id={run}")).await,
        json!([])
    );
    let detail = get(&client, &format!("/runs/{run}")).await;
    assert_eq!(detail["steps"][1]["output"]["review_item_id"], json!(id));

    let listed = get(&client, "/review-items").await;
    assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
    assert_eq!(listed[0]["id"], json!(id));
    assert_eq!(listed[0]["kind"], "tidy.note");
}

#[sqlx::test]
async fn dismiss_resolves_a_review_item(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, _) = parked(&client, &mut subscriber, application, 0.3).await;
    let listed = get(&client, "/review-items").await;
    let id = listed[0]["id"].as_i64().expect("id");
    assert_eq!(listed[0]["run_id"], json!(run));
    let payload = json!({"application_id": application, "note": "Ask about the team"});

    let (status, item) = post(
        &client,
        &format!("/review-items/{id}/dismiss"),
        Some(json!({"reason": "  Not relevant  "})),
    )
    .await;
    assert_eq!(status, Status::Ok, "{item}");
    assert_eq!(item["id"], json!(id));
    assert_eq!(item["status"], "dismissed");
    assert!(item["resolved_at"].is_string(), "{item}");
    assert_eq!(get(&client, "/review-items").await, json!([]));
    let recorded = signals(&client).await;
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    let signal = &recorded[0];
    assert_eq!(
        (
            &signal["kind"],
            &signal["run_id"],
            &signal["review_item_id"],
            &signal["pending_action_id"],
            &signal["subject"],
            &signal["reason"],
        ),
        (
            &json!("dismissal"),
            &json!(run),
            &json!(id),
            &Value::Null,
            &json!("tidy.note"),
            &json!("Not relevant"),
        )
    );
    assert_eq!(signal["drafted"], payload);
    assert_eq!(
        (&signal["approved"], &signal["changes"]),
        (&Value::Null, &Value::Null)
    );

    // No reason, and a blank one, are both stored as `null`.
    for body in [json!({}), json!({"reason": "   "})] {
        parked(&client, &mut subscriber, application, 0.3).await;
        let other = get(&client, "/review-items").await[0]["id"].clone();
        let (status, _) = post(
            &client,
            &format!("/review-items/{other}/dismiss"),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, Status::Ok, "{body}");
        let newest = &signals(&client).await[0];
        assert_eq!(newest["review_item_id"], other, "{body}");
        assert_eq!(newest["reason"], Value::Null, "{body}");
    }
    let count = signal_count(&pool).await;
    assert_eq!(count, 3);

    let stored = || async {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, resolved_at::text FROM review_items WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("read the item")
    };
    let before = stored().await;
    assert_eq!(before.0, "dismissed");
    let (status, _) = post(
        &client,
        &format!("/review-items/{id}/dismiss"),
        Some(json!({"reason": "again"})),
    )
    .await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(stored().await, before, "the second dismiss changed nothing");
    assert_eq!(signal_count(&pool).await, count, "no new signal");

    let (status, _) = post(
        &client,
        &format!("/review-items/{}/dismiss", i64::MAX),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::NotFound);
    assert_eq!(
        signal_count(&pool).await,
        count,
        "no signal for a missing item"
    );
}

#[sqlx::test]
async fn dismiss_validates_its_body(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    parked(&client, &mut subscriber, application, 0.3).await;
    let id = get(&client, "/review-items").await[0]["id"].clone();
    let path = format!("/review-items/{id}/dismiss");

    let (status, body) = post(&client, &path, Some(json!({"reason": "x".repeat(1001)}))).await;
    assert_eq!(status, Status::UnprocessableEntity);
    assert_eq!(
        body,
        json!({"errors": [{"field": "reason", "message": "At most 1000 characters"}]})
    );
    for body in [
        json!({"extra": 1}),
        json!({"reason": 5}),
        json!({"reason": ["why"]}),
    ] {
        let (status, _) = post(&client, &path, Some(body.clone())).await;
        assert_eq!(status, Status::BadRequest, "{body}");
    }
    let response = client
        .post(path.clone())
        .header(host())
        .header(ContentType::JSON)
        .body("{")
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::BadRequest, "malformed JSON");
    let (status, _) = post(&client, &path, None).await;
    assert_eq!(status, Status::UnsupportedMediaType, "no content type");
    let response = client
        .post(path.clone())
        .header(host())
        .header(ContentType::Plain)
        .body("{}")
        .dispatch()
        .await;
    assert_eq!(
        response.status(),
        Status::UnsupportedMediaType,
        "text/plain"
    );

    let items = get(&client, "/review-items").await;
    assert_eq!(items[0]["id"], id);
    assert_eq!(items[0]["status"], "pending", "nothing was dismissed");
    assert_eq!(signal_count(&pool).await, 0, "no signal was written");

    // 1000 characters is the limit, not past it.
    let (status, _) = post(&client, &path, Some(json!({"reason": "x".repeat(1000)}))).await;
    assert_eq!(status, Status::Ok);
    assert_eq!(signals(&client).await[0]["reason"], json!("x".repeat(1000)));
}

#[sqlx::test]
async fn undo_refuses_a_changed_record(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, _) = parked(&client, &mut subscriber, application, 1.0).await;
    let logged = get(&client, &format!("/actions?run_id={run}")).await;
    let id = logged[0]["id"].as_i64().expect("id");

    sqlx::query("DELETE FROM application_events WHERE kind = 'note' AND application_id = $1")
        .bind(application)
        .execute(&pool)
        .await
        .expect("delete the note");
    let (status, _) = post(&client, &format!("/actions/{id}/undo"), None).await;
    assert_eq!(status, Status::Conflict);
    let after = get(&client, &format!("/actions?run_id={run}")).await;
    assert_eq!(after[0]["undone_at"], Value::Null);

    let missing = i64::MAX;
    for (path, body) in [
        (
            format!("/pending-actions/{missing}/approve"),
            Some(json!({})),
        ),
        (format!("/pending-actions/{missing}/reject"), None),
        (format!("/actions/{missing}/undo"), None),
    ] {
        let (status, _) = post(&client, &path, body).await;
        assert_eq!(status, Status::NotFound, "{path}");
    }
}

async fn run_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM agent_runs")
        .fetch_one(pool)
        .await
        .expect("count runs")
}

#[sqlx::test]
async fn tidy_route_validates_its_body(pool: PgPool) {
    let (client, _subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let path = format!("/applications/{application}/tidy");
    let valid = json!({"note": "n", "message": "m"});

    let cases: Vec<(String, Value, Status, Option<&str>)> = vec![
        (
            format!("/applications/{}/tidy", application + 1),
            valid.clone(),
            Status::NotFound,
            None,
        ),
        (
            path.clone(),
            json!({"note": "  ", "message": "m"}),
            Status::UnprocessableEntity,
            Some("note"),
        ),
        (
            path.clone(),
            json!({"note": "n", "message": ""}),
            Status::UnprocessableEntity,
            Some("message"),
        ),
        (
            path.clone(),
            json!({"note": "n", "message": "m", "confidence": 1.5}),
            Status::UnprocessableEntity,
            Some("confidence"),
        ),
        (
            path.clone(),
            json!({"note": "n", "message": "m", "confidence": -0.1}),
            Status::UnprocessableEntity,
            Some("confidence"),
        ),
        (
            path.clone(),
            json!({"note": "n", "message": "m", "extra": 1}),
            Status::BadRequest,
            None,
        ),
    ];
    for (path, body, expected, field) in cases {
        let (status, response) = post(&client, &path, Some(body.clone())).await;
        assert_eq!(status, expected, "{path} {body}");
        if let Some(field) = field {
            assert_eq!(response["errors"][0]["field"], field, "{body}");
            assert_eq!(response["errors"].as_array().map(Vec::len), Some(1));
        }
    }
    let response = client
        .post(path.clone())
        .header(host())
        .header(ContentType::Plain)
        .body(valid.to_string())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::UnsupportedMediaType);
    assert_eq!(run_count(&pool).await, 0, "no request created a run");

    let (status, run) = post(&client, &path, Some(valid)).await;
    assert_eq!(status, Status::Created);
    assert_eq!(
        run["input"],
        json!({"application_id": application, "note": "n", "message": "m", "confidence": 1.0})
    );
}

#[sqlx::test]
async fn changing_routes_refuse_cross_site_and_forms(pool: PgPool) {
    let (client, mut subscriber) = app(&pool).await;
    let application = create_application(&client).await;
    let (run, action) = parked(&client, &mut subscriber, application, 1.0).await;
    let logged = get(&client, &format!("/actions?run_id={run}")).await;
    let undo = format!("/actions/{}/undo", logged[0]["id"]);
    let tidy = format!("/applications/{application}/tidy");
    let approve = format!("/pending-actions/{action}/approve");
    let reject = format!("/pending-actions/{action}/reject");
    let (_, _) = parked(&client, &mut subscriber, application, 0.3).await;
    let review_item = get(&client, "/review-items").await[0]["id"].clone();
    let dismiss = format!("/review-items/{review_item}/dismiss");
    let json_body = json!({"note": "n", "message": "m"}).to_string();

    for path in [&approve, &reject, &undo, &tidy, &dismiss] {
        let response = client
            .post(path.clone())
            .header(host())
            .header(ContentType::JSON)
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .body(json_body.clone())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden, "cross-site {path}");

        for (form, body) in [
            (ContentType::Form, "note=n&message=m"),
            (ContentType::Form, "_method=post&note=n"),
            (ContentType::FormData, "note=n&message=m"),
        ] {
            let response = client
                .post(path.clone())
                .header(host())
                .header(form.clone())
                .body(body)
                .dispatch()
                .await;
            assert_eq!(response.status(), Status::Forbidden, "{form} {body} {path}");
        }

        // Rocket turns this form POST into a DELETE, and no DELETE route has these paths, so
        // it never reaches a handler.
        let response = client
            .post(path.clone())
            .header(host())
            .header(ContentType::Form)
            .body("_method=delete&note=n")
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::NotFound, "_method=delete {path}");
    }

    let pending = get(&client, "/pending-actions").await;
    assert_eq!(pending[0]["status"], "pending", "nothing was decided");
    let after = get(&client, &format!("/actions?run_id={run}")).await;
    assert_eq!(after, logged, "nothing was undone");
    assert_eq!(run_count(&pool).await, 2, "no run was started");
    let items = get(&client, "/review-items").await;
    assert_eq!(items[0]["id"], review_item);
    assert_eq!(items[0]["status"], "pending", "nothing was dismissed");
    assert_eq!(signal_count(&pool).await, 0, "no signal was written");
}

/// An `external` tool that counts its handler runs. With `hold`, the handler waits for
/// `release` after telling `entered` it started; with `fail`, it fails.
#[derive(Clone, Default)]
struct Mailer {
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    hold: bool,
    fail: bool,
}

#[rocket::async_trait]
impl Tool for Mailer {
    const NAME: &'static str = "send";
    const DESCRIPTION: &'static str = "Sends a message";
    const PERMISSION: Permission = Permission::External;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object"})
    }
    fn output_schema() -> Value {
        json!({"type": "object"})
    }
    async fn call(&self, _: &mut ToolContext<'_>, _: Value) -> Result<Value, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hold {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if self.fail {
            return Err(ToolError::Failed("the send failed".into()));
        }
        Ok(json!({"sent": true}))
    }
}

/// A registry with `tool`, a step to call it from, and the approved pending action of its
/// `{"to": "a"}` request: `(registry, runner, step id, pending action id)`.
async fn approved_send(pool: &PgPool, tool: Mailer) -> (Registry, ClaudeRunner, i64, i64) {
    let mut registry = Registry::default();
    registry.register(tool).expect("send registers");
    let runner = ClaudeRunner::new("claude", Duration::from_secs(1));
    let (step,): (i64,) = sqlx::query_as(
        "WITH run AS (INSERT INTO agent_runs (workflow, input) VALUES ('w', '{}') RETURNING id) \
         INSERT INTO run_steps (run_id, ordinal, name) SELECT id, 0, 'a' FROM run RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert a run and step");
    let asked = registry
        .invoke(pool, &runner, step, None, "send", json!({"to": "a"}))
        .await;
    let Err(ToolCallError::AwaitingApproval { pending_action_id }) = asked else {
        panic!("the first request waits for approval: {asked:?}");
    };
    sqlx::query(
        "UPDATE pending_actions SET status = 'approved', approved_input = input, \
         decided_at = now() WHERE id = $1",
    )
    .bind(pending_action_id)
    .execute(pool)
    .await
    .expect("approve");
    (registry, runner, step, pending_action_id)
}

async fn action_status(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM pending_actions WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read the pending action")
}

#[sqlx::test]
async fn concurrent_attempts_run_an_approved_action_once(pool: PgPool) {
    let tool = Mailer {
        hold: true,
        ..Mailer::default()
    };
    let (registry, runner, step, action) = approved_send(&pool, tool.clone()).await;
    let input = json!({"to": "a"});

    // The second attempt calls while the first is inside the handler.
    let first = registry.invoke(&pool, &runner, step, None, "send", input.clone());
    let second = async {
        tool.entered.notified().await;
        let result = registry
            .invoke(&pool, &runner, step, None, "send", input.clone())
            .await;
        tool.release.notify_one();
        result
    };
    let (first, second) = tokio::time::timeout(TIMEOUT, async { tokio::join!(first, second) })
        .await
        .expect("both attempts finish");

    assert_eq!(
        first.expect("the first attempt runs it").0,
        json!({"sent": true})
    );
    assert!(
        matches!(second, Err(ToolCallError::Executing { pending_action_id }) if pending_action_id == action),
        "the second attempt doesn't run it: {second:?}"
    );
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1, "the handler ran once");
    assert_eq!(action_status(&pool, action).await, "executed");

    let rerun = registry
        .invoke(&pool, &runner, step, None, "send", input)
        .await
        .expect("a rerun gets the stored output");
    assert_eq!(rerun.0, json!({"sent": true}));
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1, "still once");
}

#[sqlx::test]
async fn a_failed_approved_action_never_runs_again(pool: PgPool) {
    let tool = Mailer {
        fail: true,
        ..Mailer::default()
    };
    let (registry, runner, step, action) = approved_send(&pool, tool.clone()).await;
    let input = json!({"to": "a"});

    let first = registry
        .invoke(&pool, &runner, step, None, "send", input.clone())
        .await;
    assert!(matches!(first, Err(ToolCallError::Failed(_))), "{first:?}");
    assert_eq!(action_status(&pool, action).await, "failed");

    let rerun = registry
        .invoke(&pool, &runner, step, None, "send", input)
        .await;
    assert!(
        matches!(rerun, Err(ToolCallError::ExecutionFailed { pending_action_id }) if pending_action_id == action),
        "a rerun doesn't run it again: {rerun:?}"
    );
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1, "the handler ran once");
}
