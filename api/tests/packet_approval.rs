//! Acting on an application packet end to end: the `approve_packet` run that parks a
//! `ready` packet at the approval gate, approving it (with edits) or rejecting it, the
//! review interplay, marking it applied through the `apply_packet` run, both undos and the
//! two packet routes. Nothing is ever submitted.

mod common;

use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::events::{Event, EventBus, Subscriber};

const TIMEOUT: Duration = Duration::from_secs(20);
const DESCRIPTION: &str = "Build our Rust services on PostgreSQL.";

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The API with the production runtime and a runner playing `mode`, and a subscriber to its
/// bus. Keep the `Fake` alive as long as the client.
async fn app(pool: &PgPool, mode: &str) -> (Client, common::Fake, Subscriber) {
    let fake = common::fake(mode, Duration::from_secs(30));
    let rocket = common::app(pool.clone(), fake.runner.clone(), common::fast_config());
    let client = Client::tracked(rocket).await.expect("valid rocket");
    let subscriber = client
        .rocket()
        .state::<EventBus>()
        .expect("the bus")
        .subscribe();
    (client, fake, subscriber)
}

/// `(status, JSON body or null)` of a request.
async fn send(client: &Client, post: Option<Value>, path: &str) -> (Status, Value) {
    let request = match &post {
        Some(body) => client
            .post(path.to_owned())
            .header(ContentType::JSON)
            .body(body.to_string()),
        None => client.get(path.to_owned()),
    };
    let response = request.header(host()).dispatch().await;
    let status = response.status();
    let body = response.into_json().await.unwrap_or(Value::Null);
    (status, body)
}

async fn get(client: &Client, path: &str) -> Value {
    let (status, body) = send(client, None, path).await;
    assert_eq!(status, Status::Ok, "GET {path}: {body}");
    body
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"))
}

/// Waits until run `id` is `status`, and returns `(output, error)`.
async fn until(pool: &PgPool, id: i64, status: &str) -> (Value, Option<String>) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let (current, output, error): (String, Option<Value>, Option<String>) =
            sqlx::query_as("SELECT status, output, error FROM agent_runs WHERE id = $1")
                .bind(id)
                .fetch_one(pool)
                .await
                .expect("read the run");
        if current == status {
            return (output.unwrap_or(Value::Null), error);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "run {id} is {current}, never {status}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits for packet `packet`'s newest `workflow` run to exist and returns its id.
async fn run_for(pool: &PgPool, workflow: &str, packet: i64) -> i64 {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT max(id) FROM agent_runs WHERE workflow = $1 \
             AND input -> 'packet_id' = to_jsonb($2::bigint)",
        )
        .bind(workflow)
        .bind(packet)
        .fetch_one(pool)
        .await
        .expect("read the runs");
        if let Some(id) = found {
            return id;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "packet {packet} never got a {workflow} run"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A posting whose description mentions Rust; returns its id.
async fn seed_posting(pool: &PgPool, company: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO job_postings (source, board, external_id, title, company, url, \
         location_raw, remote, description) \
         VALUES ('greenhouse', 'acme', $1, 'Backend Engineer', $1, 'https://example.com/job', \
         'Remote - EU', true, $2) RETURNING id",
    )
    .bind(company)
    .bind(DESCRIPTION)
    .fetch_one(pool)
    .await
    .expect("insert a posting")
}

/// An applied fit score for `posting`.
async fn seed_applied(pool: &PgPool, posting: i64) {
    sqlx::query(
        "WITH run AS (INSERT INTO agent_runs (workflow, input, status) \
           VALUES ('score_fit', jsonb_build_object('posting_id', $1::bigint), 'succeeded') \
           RETURNING id) \
         INSERT INTO posting_fit_scores (posting_id, run_id, status, score, band, confidence, \
         summary, reasons) SELECT $1, id, 'applied', 75, 'partial', 0.9, 'Earlier', '[]' FROM run",
    )
    .bind(posting)
    .execute(pool)
    .await
    .expect("insert a score");
}

/// Resume 1 "Backend" and resume 2 "Frontend", and confirmed memories 1 to 5 (the
/// `prep_packet` fixture cites them).
async fn seed_user(pool: &PgPool) {
    for (name, text) in [
        ("Backend", "Backend engineer: Rust services and PostgreSQL"),
        ("Frontend", "Frontend engineer: TypeScript and Svelte"),
    ] {
        sqlx::query(
            "INSERT INTO resume_versions (name, filename, size_bytes, sha256, pdf, text) \
             VALUES ($1, $1 || '.pdf', 9, md5($1), $2, $3)",
        )
        .bind(name)
        .bind(b"%PDF-test".as_slice())
        .bind(text)
        .execute(pool)
        .await
        .expect("seed a resume");
    }
    sqlx::query(
        "INSERT INTO memories (kind, topic, content, status, confirmed_at) VALUES \
         ('answer', 'work_authorization', 'EU citizen, no sponsorship needed', 'confirmed', now()), \
         ('answer', 'notice_period', 'Four weeks notice', 'confirmed', now()), \
         ('answer', 'salary_expectation', 'Around 90,000 EUR a year', 'confirmed', now()), \
         ('answer', 'why_leaving', 'Wants to own a larger platform', 'confirmed', now()), \
         ('fact', 'skill', 'Shipped Rust services', 'confirmed', now())",
    )
    .execute(pool)
    .await
    .expect("seed memories");
}

/// A scored posting at Acme with resumes and memories; returns the posting.
async fn seed_all(pool: &PgPool) -> i64 {
    seed_user(pool).await;
    let posting = seed_posting(pool, "Acme").await;
    seed_applied(pool, posting).await;
    posting
}

/// Prepares a packet for `posting`; returns `(prep run, packet)` once the run succeeded.
async fn prepared(client: &Client, pool: &PgPool, posting: i64) -> (i64, i64) {
    let (status, body) = send(
        client,
        Some(json!({})),
        &format!("/postings/{posting}/packet"),
    )
    .await;
    assert_eq!(status, Status::Accepted, "{body}");
    let run = body["run"]["id"].as_i64().expect("run id");
    let (output, _) = until(pool, run, "succeeded").await;
    (run, output["packet_id"].as_i64().expect("a packet"))
}

/// Waits for packet `packet`'s approval to park; returns `(approve run, pending action)`.
async fn parked(pool: &PgPool, packet: i64) -> (i64, i64) {
    let run = run_for(pool, "approve_packet", packet).await;
    until(pool, run, "awaiting_approval").await;
    let action: i64 = sqlx::query_scalar(
        "SELECT id FROM pending_actions WHERE run_id = $1 AND status = 'pending'",
    )
    .bind(run)
    .fetch_one(pool)
    .await
    .expect("one pending action");
    (run, action)
}

/// A ready packet for a new scored posting, approved as drafted: `(posting, packet)`.
async fn approved_packet(client: &Client, pool: &PgPool) -> (i64, i64) {
    let posting = seed_all(pool).await;
    let (_, packet) = prepared(client, pool, posting).await;
    let (run, action) = parked(pool, packet).await;
    let (status, body) = send(
        client,
        Some(json!({})),
        &format!("/pending-actions/{action}/approve"),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    until(pool, run, "succeeded").await;
    (posting, packet)
}

/// Marks `packet` applied and waits for the run; returns the run's output.
async fn mark_applied(client: &Client, pool: &PgPool, packet: i64) -> Value {
    let (status, body) = send(
        client,
        Some(json!({})),
        &format!("/packets/{packet}/applied"),
    )
    .await;
    assert_eq!(status, Status::Accepted, "{body}");
    assert_eq!(body["reused"], false);
    assert_eq!(body["run"]["workflow"], "apply_packet");
    let run = body["run"]["id"].as_i64().expect("run id");
    until(pool, run, "succeeded").await.0
}

fn memory_citation(id: i64, topic: &str, text: &str) -> Value {
    json!({"source": "memory", "memory_id": id, "topic": topic, "text": text})
}

/// The five answers of the `prep_packet` fixture, as stored.
fn stored_answers() -> Value {
    json!([
        {"key": "work_authorization",
         "question": "Are you authorized to work in the role's location, and will you need visa sponsorship?",
         "answer": "Yes. I am an EU citizen and need no visa sponsorship.",
         "citations": [memory_citation(1, "work_authorization", "EU citizen, no sponsorship needed")]},
        {"key": "notice_period", "question": "When could you start?",
         "answer": "I can start four weeks after an offer.",
         "citations": [memory_citation(2, "notice_period", "Four weeks notice")]},
        {"key": "salary_expectation", "question": "What are your salary expectations?",
         "answer": "Around 90,000 EUR a year.",
         "citations": [memory_citation(3, "salary_expectation", "Around 90,000 EUR a year")]},
        {"key": "why_leaving", "question": "Why are you looking to leave your current role?",
         "answer": "I want to own a larger platform.",
         "citations": [memory_citation(4, "why_leaving", "Wants to own a larger platform")]},
        {"key": "why_company", "question": "Why do you want to work at Acme?",
         "answer": "Your team builds Rust services, which is what I do best.",
         "citations": [memory_citation(5, "skill", "Shipped Rust services")]},
    ])
}

/// The approval request for the fixture's draft.
fn draft(packet: i64) -> Value {
    json!({"packet_id": packet, "resume_version_id": 1, "answers": {
        "work_authorization": "Yes. I am an EU citizen and need no visa sponsorship.",
        "notice_period": "I can start four weeks after an offer.",
        "salary_expectation": "Around 90,000 EUR a year.",
        "why_leaving": "I want to own a larger platform.",
        "why_company": "Your team builds Rust services, which is what I do best.",
    }})
}

async fn packet_status(pool: &PgPool, packet: i64) -> String {
    sqlx::query_scalar("SELECT status FROM application_packets WHERE id = $1")
        .bind(packet)
        .fetch_one(pool)
        .await
        .expect("read the packet")
}

async fn action_status(pool: &PgPool, action: i64) -> String {
    sqlx::query_scalar("SELECT status FROM pending_actions WHERE id = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("read the pending action")
}

/// The not-undone action of `tool` for `packet`.
async fn action_of(pool: &PgPool, tool: &str, packet: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT max(id) FROM agent_actions WHERE tool = $1 AND undone_at IS NULL \
         AND undo_data -> 'packet_id' = to_jsonb($2::bigint)",
    )
    .bind(tool)
    .bind(packet)
    .fetch_one(pool)
    .await
    .expect("read the actions")
}

const SIGNALS: &str = "SELECT count(*) FROM memory_signals";

#[sqlx::test]
async fn ready_packet_is_approved_with_edits_and_one_signal(pool: PgPool) {
    let posting = seed_all(&pool).await;
    let (client, _fake, _) = app(&pool, "prep_packet").await;
    let (_, packet) = prepared(&client, &pool, posting).await;

    let (run, action) = parked(&pool, packet).await;
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'approve_packet'"
        )
        .await,
        1
    );
    let pending = get(&client, "/pending-actions").await;
    assert_eq!(pending.as_array().map(Vec::len), Some(1), "{pending}");
    assert_eq!(pending[0]["id"], action);
    assert_eq!(pending[0]["tool"], "approve_application_packet");
    assert_eq!(pending[0]["input"], draft(packet));
    let shown = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(shown["status"], "ready");
    assert_eq!(shown["pending_action_id"], action);
    assert_eq!(shown["undo_action_id"], Value::Null);
    assert_eq!(
        shown["latest_run"],
        json!({"id": run, "status": "awaiting_approval", "error": null})
    );

    let mut edited = draft(packet);
    edited["resume_version_id"] = json!(2);
    edited["answers"]["notice_period"] = json!("I can start in two weeks.");
    let (status, body) = send(
        &client,
        Some(json!({"input": edited})),
        &format!("/pending-actions/{action}/approve"),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let (output, _) = until(&pool, run, "succeeded").await;
    assert_eq!(output, json!({"packet_id": packet, "approved": true}));

    type Row = (
        String,
        Option<i64>,
        Value,
        Option<i64>,
        Option<String>,
        Value,
        bool,
    );
    let row: Row = sqlx::query_as(
        "SELECT status, resume_version_id, answers, approved_resume_version_id, \
         approved_resume_name, approved_answers, approved_at IS NOT NULL \
         FROM application_packets WHERE id = $1",
    )
    .bind(packet)
    .fetch_one(&pool)
    .await
    .expect("read the packet");
    let mut approved = stored_answers();
    approved[1]["answer"] = json!("I can start in two weeks.");
    approved[1]["citations"] = json!([]);
    assert_eq!(
        row,
        (
            "approved".into(),
            Some(1),
            stored_answers(),
            Some(2),
            Some("Frontend".into()),
            approved.clone(),
            true,
        )
    );

    assert_eq!(count(&pool, SIGNALS).await, 1);
    let (kind, changes): (String, Value) =
        sqlx::query_as("SELECT kind, changes FROM memory_signals WHERE pending_action_id = $1")
            .bind(action)
            .fetch_one(&pool)
            .await
            .expect("the approval signal");
    assert_eq!(kind, "approval");
    assert_eq!(
        changes,
        json!([
            {"op": "replace", "path": "/answers/notice_period",
             "before": "I can start four weeks after an offer.",
             "after": "I can start in two weeks."},
            {"op": "replace", "path": "/resume_version_id", "before": 1, "after": 2},
        ])
    );

    let approval = action_of(&pool, "approve_application_packet", packet).await;
    let shown = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(shown["status"], "approved");
    assert_eq!(shown["approved_resume_version_id"], 2);
    assert_eq!(shown["approved_resume_name"], "Frontend");
    assert_eq!(shown["approved_answers"], approved);
    assert!(shown["approved_at"].is_string(), "{shown}");
    assert_eq!(shown["resume_version_id"], 1);
    assert_eq!(shown["answers"], stored_answers());
    assert_eq!(shown["pending_action_id"], Value::Null);
    assert_eq!(shown["undo_action_id"], approval);
    assert_eq!(
        shown["latest_run"],
        json!({"id": run, "status": "succeeded", "error": null})
    );
    let listed = get(&client, &format!("/postings/{posting}/packets")).await;
    assert_eq!(listed["packets"][0], shown);
}

#[sqlx::test]
async fn review_comes_before_approval_and_bad_edits_are_refused(pool: PgPool) {
    let posting = seed_all(&pool).await;
    let (client, _fake, _) = app(&pool, "prep_packet_missing").await;
    let (run, packet) = prepared(&client, &pool, posting).await;
    let approvals = "SELECT count(*) FROM agent_runs WHERE workflow = 'approve_packet'";

    assert_eq!(packet_status(&pool, packet).await, "in_review");
    assert_eq!(count(&pool, approvals).await, 0);
    let shown = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(shown["pending_action_id"], Value::Null);
    assert_eq!(shown["latest_run"], Value::Null);
    let (status, _) = send(
        &client,
        Some(json!({})),
        &format!("/packets/{packet}/approval"),
    )
    .await;
    assert_eq!(
        status,
        Status::Conflict,
        "an in_review packet can't be approved"
    );

    let (output, _) = until(&pool, run, "succeeded").await;
    let item = output["review_item_id"].as_i64().expect("a review item");
    let (status, body) = send(
        &client,
        Some(json!({})),
        &format!("/review-items/{item}/accept"),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(
        count(&pool, SIGNALS).await,
        0,
        "accepting records no signal"
    );
    assert_eq!(count(&pool, approvals).await, 1);
    let (approval, action) = parked(&pool, packet).await;
    let mut asked = draft(packet);
    asked["answers"]["salary_expectation"] = Value::Null;
    let input: Value = sqlx::query_scalar("SELECT input FROM pending_actions WHERE id = $1")
        .bind(action)
        .fetch_one(&pool)
        .await
        .expect("read the action");
    assert_eq!(input, asked);

    let approve = format!("/pending-actions/{action}/approve");
    let refusal = |edit: Value| {
        let mut edited = asked.clone();
        for (key, value) in edit.as_object().expect("an object") {
            if key == "answers" {
                for (answer, text) in value.as_object().expect("answers") {
                    edited["answers"][answer] = text.clone();
                }
            } else {
                edited[key] = value.clone();
            }
        }
        json!({"input": edited})
    };
    for (edit, message) in [
        (
            json!({"resume_version_id": 999}),
            "$.resume_version_id: no resume has this id",
        ),
        (
            json!({"resume_version_id": null}),
            "$.resume_version_id: choose a resume",
        ),
        (
            json!({"packet_id": packet + 1}),
            "$.packet_id: can't be changed",
        ),
        (
            json!({"answers": {"why_leaving": "a".repeat(1_501)}}),
            "$.answers.why_leaving: at most 1500 characters",
        ),
    ] {
        let (status, body) = send(&client, Some(refusal(edit.clone())), &approve).await;
        assert_eq!(status, Status::UnprocessableEntity, "{edit}: {body}");
        assert_eq!(
            body,
            json!({"errors": [{"field": "input", "message": message}]}),
            "{edit}"
        );
        assert_eq!(action_status(&pool, action).await, "pending", "{edit}");
    }
    // At most 1,500 after trimming is fine; it's refused below for another reason.
    sqlx::query("UPDATE application_packets SET status = 'dismissed' WHERE id = $1")
        .bind(packet)
        .execute(&pool)
        .await
        .expect("dismiss the packet");
    let (status, _) = send(
        &client,
        Some(refusal(
            json!({"answers": {"why_leaving": format!(" {} ", "a".repeat(1_500))}}),
        )),
        &approve,
    )
    .await;
    assert_eq!(status, Status::Conflict, "a dismissed packet");
    let (status, _) = send(&client, Some(json!({})), &approve).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(action_status(&pool, action).await, "pending");
    assert_eq!(
        count(&pool, SIGNALS).await,
        0,
        "no refusal records a signal"
    );

    sqlx::query("UPDATE application_packets SET status = 'ready' WHERE id = $1")
        .bind(packet)
        .execute(&pool)
        .await
        .expect("make the packet ready again");
    let (status, body) = send(&client, Some(json!({})), &approve).await;
    assert_eq!(status, Status::Ok, "{body}");
    until(&pool, approval, "succeeded").await;
    assert_eq!(packet_status(&pool, packet).await, "approved");
    assert_eq!(count(&pool, SIGNALS).await, 1, "one signal in total");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memory_signals WHERE kind = 'approval'"
        )
        .await,
        1
    );
}

#[sqlx::test]
async fn rejecting_the_approval_dismisses_the_packet(pool: PgPool) {
    let posting = seed_all(&pool).await;
    let (client, _fake, _) = app(&pool, "prep_packet").await;
    let (_, packet) = prepared(&client, &pool, posting).await;
    let (run, action) = parked(&pool, packet).await;

    // Reject takes no body.
    let response = client
        .post(format!("/pending-actions/{action}/reject"))
        .header(host())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok);
    until(&pool, run, "rejected").await;
    assert_eq!(packet_status(&pool, packet).await, "dismissed");
    assert_eq!(action_status(&pool, action).await, "rejected");
    assert_eq!(count(&pool, SIGNALS).await, 1);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memory_signals WHERE kind = 'rejection' \
             AND subject = 'approve_application_packet'"
        )
        .await,
        1
    );
    let shown = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(shown["pending_action_id"], Value::Null);
    assert_eq!(shown["latest_run"]["status"], "rejected");
}

#[sqlx::test]
async fn mark_applied_creates_an_application_and_undo_deletes_it(pool: PgPool) {
    let (client, _fake, mut subscriber) = app(&pool, "prep_packet").await;
    let (_, packet) = approved_packet(&client, &pool).await;

    let output = mark_applied(&client, &pool, packet).await;
    let application = output["application_id"].as_i64().expect("an application");
    assert_eq!(
        output,
        json!({"packet_id": packet, "application_id": application, "created": true})
    );
    let shown = get(&client, &format!("/applications/{application}")).await;
    for (key, value) in [
        ("company", json!("Acme")),
        ("title", json!("Backend Engineer")),
        ("url", json!("https://example.com/job")),
        ("location", json!("Remote - EU")),
        ("source", json!("Greenhouse")),
        ("status", json!("applied")),
        ("resume_version_id", json!(1)),
    ] {
        assert_eq!(shown[key], value, "{key}: {shown}");
    }
    let events: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT kind, to_status FROM application_events WHERE application_id = $1")
            .bind(application)
            .fetch_all(&pool)
            .await
            .expect("read events");
    assert_eq!(events, [("created".into(), Some("applied".into()))]);
    let published = common::events(&mut subscriber, TIMEOUT, |event| {
        matches!(event, Event::ApplicationCreated { application: created } if created.id == application)
    })
    .await;
    assert!(
        matches!(published.last(), Some(Event::ApplicationCreated { .. })),
        "application.created was published"
    );

    let mark = action_of(&pool, "mark_packet_applied", packet).await;
    let packet_json = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(packet_json["status"], "applied");
    assert_eq!(packet_json["application_id"], application);
    assert!(packet_json["applied_at"].is_string());
    assert_eq!(packet_json["undo_action_id"], mark);
    assert_eq!(packet_json["latest_run"]["status"], "succeeded");

    let undo = format!("/actions/{mark}/undo");
    let (status, body) = send(&client, Some(json!({})), &undo).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(count(&pool, "SELECT count(*) FROM applications").await, 0);
    let packet_json = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(packet_json["status"], "approved");
    assert_eq!(packet_json["application_id"], Value::Null);
    assert_eq!(packet_json["applied_at"], Value::Null);
    assert_eq!(
        packet_json["undo_action_id"],
        action_of(&pool, "approve_application_packet", packet).await
    );
    let (status, _) = send(&client, Some(json!({})), &undo).await;
    assert_eq!(status, Status::Conflict, "a second undo");
}

#[sqlx::test]
async fn mark_applied_updates_the_postings_application_and_undo_restores_it(pool: PgPool) {
    let (client, _fake, _) = app(&pool, "prep_packet").await;
    let (posting, packet) = approved_packet(&client, &pool).await;

    // An earlier packet of the posting was applied with a `sourced` application.
    type Prior = (i64, String, String, Option<i64>, String);
    let prior: Prior = sqlx::query_as(
        "INSERT INTO applications (company, title, status, status_changed_at, updated_at, \
         resume_version_id) VALUES ('Acme', 'Backend', 'sourced', now() - interval '3 days', \
         now() - interval '2 days', 2) \
         RETURNING id, status, status_changed_at::text, resume_version_id, updated_at::text",
    )
    .fetch_one(&pool)
    .await
    .expect("insert an application");
    let application = prior.0;
    sqlx::query(
        "WITH run AS (INSERT INTO agent_runs (workflow, input, status) \
           VALUES ('prep_packet', jsonb_build_object('posting_id', $1::bigint), 'succeeded') \
           RETURNING id) \
         INSERT INTO application_packets (posting_id, run_id, status, resume_version_id, \
         resume_name, resume_reasons, answers, confidence, approved_resume_name, \
         approved_answers, approved_at, application_id, applied_at) \
         SELECT $1, id, 'applied', 2, 'Frontend', '[]', '[]', 0.9, 'Frontend', '[]', \
         now() - interval '2 days', $2, now() - interval '2 days' FROM run",
    )
    .bind(posting)
    .bind(application)
    .execute(&pool)
    .await
    .expect("insert the earlier packet");
    let read = || async {
        sqlx::query_as::<_, Prior>(
            "SELECT id, status, status_changed_at::text, resume_version_id, updated_at::text \
             FROM applications WHERE id = $1",
        )
        .bind(application)
        .fetch_one(&pool)
        .await
        .expect("read the application")
    };
    let status_events = format!(
        "SELECT count(*) FROM application_events WHERE application_id = {application} \
         AND kind = 'status_changed'"
    );

    let output = mark_applied(&client, &pool, packet).await;
    assert_eq!(
        output,
        json!({"packet_id": packet, "application_id": application, "created": false})
    );
    let now = read().await;
    assert_eq!((now.1.as_str(), now.3), ("applied", Some(1)));
    assert_ne!(now.2, prior.2);
    let event: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT from_status, to_status FROM application_events WHERE application_id = $1 \
         AND kind = 'status_changed'",
    )
    .bind(application)
    .fetch_one(&pool)
    .await
    .expect("one status event");
    assert_eq!(event, (Some("sourced".into()), Some("applied".into())));
    assert_eq!(
        get(&client, &format!("/packets/{packet}")).await["application_id"],
        application
    );

    let mark = action_of(&pool, "mark_packet_applied", packet).await;
    let (status, body) = send(&client, Some(json!({})), &format!("/actions/{mark}/undo")).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(read().await, prior, "restored exactly");
    assert_eq!(count(&pool, &status_events).await, 0);
    assert_eq!(packet_status(&pool, packet).await, "approved");

    // Applied again, then the application changes: undo refuses and changes nothing.
    mark_applied(&client, &pool, packet).await;
    let mark = action_of(&pool, "mark_packet_applied", packet).await;
    sqlx::query("UPDATE applications SET notes = 'Called', updated_at = now() WHERE id = $1")
        .bind(application)
        .execute(&pool)
        .await
        .expect("edit the application");
    let edited = read().await;
    let (status, _) = send(&client, Some(json!({})), &format!("/actions/{mark}/undo")).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(read().await, edited);
    assert_eq!(count(&pool, &status_events).await, 1);
    assert_eq!(packet_status(&pool, packet).await, "applied");
}

#[sqlx::test]
async fn approval_undo_and_request_approval(pool: PgPool) {
    let (client, _fake, _) = app(&pool, "prep_packet").await;
    let (_, packet) = approved_packet(&client, &pool).await;

    let approval = action_of(&pool, "approve_application_packet", packet).await;
    let shown = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(shown["undo_action_id"], approval);
    let (status, body) = send(
        &client,
        Some(json!({})),
        &format!("/actions/{approval}/undo"),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let shown = get(&client, &format!("/packets/{packet}")).await;
    assert_eq!(shown["status"], "ready");
    for key in [
        "approved_resume_version_id",
        "approved_resume_name",
        "approved_answers",
        "approved_at",
        "undo_action_id",
        "pending_action_id",
    ] {
        assert_eq!(shown[key], Value::Null, "{key}");
    }
    assert_eq!(shown["resume_version_id"], 1);
    assert_eq!(shown["answers"], stored_answers());

    // No approval is queued by the undo; the route queues one and reuses a waiting one.
    let path = format!("/packets/{packet}/approval");
    let (status, first) = send(&client, Some(json!({})), &path).await;
    assert_eq!(status, Status::Accepted, "{first}");
    assert_eq!(first["reused"], false);
    assert_eq!(first["run"]["workflow"], "approve_packet");
    let run = first["run"]["id"].as_i64().expect("run id");
    until(&pool, run, "awaiting_approval").await;
    let (status, second) = send(&client, Some(json!({})), &path).await;
    assert_eq!(status, Status::Accepted, "{second}");
    assert_eq!(second["reused"], true);
    assert_eq!(second["run"]["id"], run);

    // Approved and applied again, the newer approval can't be undone.
    let (_, action) = parked(&pool, packet).await;
    let (status, _) = send(
        &client,
        Some(json!({})),
        &format!("/pending-actions/{action}/approve"),
    )
    .await;
    assert_eq!(status, Status::Ok);
    until(&pool, run, "succeeded").await;
    mark_applied(&client, &pool, packet).await;
    let approval = action_of(&pool, "approve_application_packet", packet).await;
    let (status, _) = send(
        &client,
        Some(json!({})),
        &format!("/actions/{approval}/undo"),
    )
    .await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(packet_status(&pool, packet).await, "applied");
    assert_eq!(count(&pool, "SELECT count(*) FROM applications").await, 1);
}

/// A packet for `posting` in `status`, prepared by a finished run of its own, with an
/// approved resume when `approved_resume` is set: its id.
async fn seed_packet(
    pool: &PgPool,
    posting: i64,
    status: &str,
    approved_resume: Option<i64>,
) -> i64 {
    sqlx::query_scalar(
        "WITH run AS (INSERT INTO agent_runs (workflow, input, status) \
           VALUES ('prep_packet', jsonb_build_object('posting_id', $1::bigint), 'succeeded') \
           RETURNING id) \
         INSERT INTO application_packets (posting_id, run_id, status, resume_version_id, \
         resume_name, resume_reasons, answers, confidence, approved_resume_version_id, \
         approved_resume_name, approved_answers, approved_at) \
         SELECT $1, id, $2, 1, 'Backend', '[]', '[]', 0.9, $3, \
         CASE WHEN $2 = 'approved' THEN 'Backend' END, \
         CASE WHEN $2 = 'approved' THEN '[]'::jsonb END, \
         CASE WHEN $2 = 'approved' THEN now() END FROM run RETURNING id",
    )
    .bind(posting)
    .bind(status)
    .bind(approved_resume)
    .fetch_one(pool)
    .await
    .expect("insert a packet")
}

#[sqlx::test]
async fn packet_action_routes_validate_and_refuse_cross_site_and_forms(pool: PgPool) {
    seed_user(&pool).await;
    let posting = seed_posting(&pool, "Acme").await;
    let in_review = seed_packet(&pool, posting, "in_review", None).await;
    let ready = seed_packet(&pool, posting, "ready", None).await;
    let approved = seed_packet(&pool, posting, "approved", Some(1)).await;
    let resume_gone = seed_packet(&pool, posting, "approved", None).await;
    // Paused, so queued runs wait.
    sqlx::query("UPDATE kill_switch SET paused = true")
        .execute(&pool)
        .await
        .expect("pause");
    let (client, _fake, _) = app(&pool, "prep_packet").await;
    let runs =
        "SELECT count(*) FROM agent_runs WHERE workflow IN ('approve_packet', 'apply_packet')";

    for (route, good, wrong) in [
        ("approval", ready, vec![in_review, approved]),
        ("applied", approved, vec![in_review, ready, resume_gone]),
    ] {
        let path = format!("/packets/{good}/{route}");
        let (status, _) = send(&client, Some(json!({})), &format!("/packets/999/{route}")).await;
        assert_eq!(status, Status::NotFound, "{route}");
        for packet in wrong {
            let (status, _) = send(
                &client,
                Some(json!({})),
                &format!("/packets/{packet}/{route}"),
            )
            .await;
            assert_eq!(status, Status::Conflict, "{route} {packet}");
        }
        let (status, _) = send(&client, Some(json!({"force": true})), &path).await;
        assert_eq!(status, Status::BadRequest, "{route}");
        let response = client
            .post(path.clone())
            .header(host())
            .header(ContentType::Plain)
            .body("{}")
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::UnsupportedMediaType, "{route}");
        let response = client
            .post(path.clone())
            .header(host())
            .header(ContentType::JSON)
            .header(Header::new("Sec-Fetch-Site", "cross-site"))
            .body("{}")
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden, "{route} cross-site");
        for (form, body) in [
            (ContentType::Form, "a=b"),
            (ContentType::Form, "_method=post&a=b"),
            (ContentType::FormData, "a=b"),
        ] {
            let response = client
                .post(path.clone())
                .header(host())
                .header(form.clone())
                .body(body)
                .dispatch()
                .await;
            assert_eq!(
                response.status(),
                Status::Forbidden,
                "{route} {form} {body}"
            );
        }
        assert_eq!(count(&pool, runs).await, 0, "{route}: no side effect");
    }

    for (route, good, workflow) in [
        ("approval", ready, "approve_packet"),
        ("applied", approved, "apply_packet"),
    ] {
        let path = format!("/packets/{good}/{route}");
        let (status, first) = send(&client, Some(json!({})), &path).await;
        assert_eq!(status, Status::Accepted, "{first}");
        assert_eq!(first["reused"], false);
        assert_eq!(first["run"]["workflow"], workflow);
        assert_eq!(first["run"]["status"], "paused");
        assert_eq!(
            first["run"]["input"],
            json!({"posting_id": posting, "packet_id": good})
        );
        let (status, second) = send(&client, Some(json!({})), &path).await;
        assert_eq!(status, Status::Accepted, "{second}");
        assert_eq!(second["reused"], true);
        assert_eq!(second["run"]["id"], first["run"]["id"]);
    }
    assert_eq!(count(&pool, runs).await, 2);
}
