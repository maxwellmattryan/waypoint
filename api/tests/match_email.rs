//! The `match_email` workflow end to end: the worker matching stored classifications to
//! applications, the `write` tools and their undo, the review items for unsure matches,
//! accepting them, and the pipeline from fixture import to application updates.

mod common;

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::events::EventBus;
use waypoint_api::jobs::Wake;
use waypoint_api::runs;
use waypoint_api::workflows::{self, classify_email, match_email};

const TIMEOUT: Duration = Duration::from_secs(20);
/// Every seeded email's `sent_at`.
const SENT_AT: &str = "2026-10-20T16:00:00Z";

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

/// The API with the production runtime and a runner playing `mode`. Keep the `Fake` alive
/// as long as the client.
async fn app_with(pool: &PgPool, mode: &str) -> (Client, common::Fake) {
    let fake = common::fake(mode, Duration::from_secs(30));
    let rocket = common::app(pool.clone(), fake.runner.clone(), common::fast_config());
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// The API for tests that make no model call: any call would hang.
async fn app(pool: &PgPool) -> (Client, common::Fake) {
    app_with(pool, "hang").await
}

async fn send(
    client: &Client,
    method: rocket::http::Method,
    path: &str,
    body: Option<Value>,
) -> (Status, Value) {
    let mut request = client.req(method, path.to_owned()).header(host());
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

async fn post(client: &Client, path: &str, body: Option<Value>) -> (Status, Value) {
    send(client, rocket::http::Method::Post, path, body).await
}

async fn put(client: &Client, path: &str, body: Value) -> Status {
    send(client, rocket::http::Method::Put, path, Some(body))
        .await
        .0
}

async fn get(client: &Client, path: &str) -> Value {
    let response = client.get(path.to_owned()).header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Ok, "GET {path}");
    response.into_json().await.expect("a JSON body")
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"))
}

/// `(application_events, agent_actions)` row counts.
async fn counts(pool: &PgPool) -> (i64, i64) {
    (
        count(pool, "SELECT count(*) FROM application_events").await,
        count(pool, "SELECT count(*) FROM agent_actions").await,
    )
}

/// Inserts an application with `status` and `stages` (no timeline events).
async fn seed_application(
    pool: &PgPool,
    company: &str,
    title: &str,
    status: &str,
    stages: &[&str],
) -> i64 {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO applications (company, title, status) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(company)
    .bind(title)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert an application");
    for (position, name) in stages.iter().enumerate() {
        sqlx::query(
            "INSERT INTO application_stages (application_id, name, position) VALUES ($1, $2, $3)",
        )
        .bind(id)
        .bind(name)
        .bind(position as i32)
        .execute(pool)
        .await
        .expect("insert a stage");
    }
    id
}

/// What the classifier said about a seeded email.
#[derive(Clone, Copy)]
struct Answer<'a> {
    class: &'a str,
    confidence: f64,
    company: Option<&'a str>,
    role: Option<&'a str>,
    stage: Option<&'a str>,
}

/// A stored email and its `classified` row.
#[derive(Clone, Copy)]
struct Seed<'a> {
    from_name: &'a str,
    from_address: &'a str,
    subject: &'a str,
    ats: Option<&'a str>,
    answer: Answer<'a>,
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant")
        .with_timezone(&Utc)
}

/// Stores email `provider_id` in a thread of its own with a `classified` row (key
/// `key-<provider_id>`). Returns `(email_id, classification_id)`.
async fn seed(pool: &PgPool, provider_id: &str, seed: Seed<'_>) -> (i64, i64) {
    let thread: i64 = sqlx::query_scalar(
        "INSERT INTO email_threads (subject, subject_key) VALUES ($1, lower($1)) RETURNING id",
    )
    .bind(seed.subject)
    .fetch_one(pool)
    .await
    .expect("insert a thread");
    let email: i64 = sqlx::query_scalar(
        "INSERT INTO emails (source, provider_message_id, thread_id, from_address, from_name, \
         subject, sent_at, headers, raw, body_text, body_source, verdict, verdict_reasons, ats, \
         normalizer_version, heuristics_version) \
         VALUES ('fixture', $1, $2, $3, $4, $5, $6, '[]', '\\x00', 'Hi', 'plain', 'candidate', \
         '[]', $7, 1, 1) RETURNING id",
    )
    .bind(provider_id)
    .bind(thread)
    .bind(seed.from_address)
    .bind(seed.from_name)
    .bind(seed.subject)
    .bind(instant(SENT_AT))
    .bind(seed.ats)
    .fetch_one(pool)
    .await
    .expect("insert an email");
    let classification = classify(pool, email, &format!("key-{provider_id}"), seed.answer).await;
    (email, classification)
}

/// Adds a `classified` row for `email` under `key`.
async fn classify(pool: &PgPool, email: i64, key: &str, answer: Answer<'_>) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO email_classifications (email_id, input_key, classifier_version, status, \
         class, confidence, company, role, stage) \
         VALUES ($1, $2, 1, 'classified', $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(email)
    .bind(key)
    .bind(answer.class)
    .bind(answer.confidence)
    .bind(answer.company)
    .bind(answer.role)
    .bind(answer.stage)
    .fetch_one(pool)
    .await
    .expect("insert a classification")
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

/// Starts a `match_email` run for `classification_id` and returns its output once it
/// succeeds.
async fn run_match(pool: &PgPool, classification_id: i64) -> Value {
    let run = runs::start(
        pool,
        &EventBus::default(),
        &Wake::default(),
        &workflows::runtime(),
        match_email::WORKFLOW,
        json!({"classification_id": classification_id}),
    )
    .await
    .expect("start a run");
    until(pool, run.id, "succeeded").await
}

/// `(status, application_id)` of a classification's match row.
async fn match_row(pool: &PgPool, classification_id: i64) -> (String, Option<i64>) {
    sqlx::query_as("SELECT status, application_id FROM email_matches WHERE classification_id = $1")
        .bind(classification_id)
        .fetch_one(pool)
        .await
        .expect("read the match row")
}

async fn status_of(pool: &PgPool, application_id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM applications WHERE id = $1")
        .bind(application_id)
        .fetch_one(pool)
        .await
        .expect("read the status")
}

/// `(name, position)` of an application's stages, in order.
async fn stages_of(client: &Client, application_id: i64) -> Vec<(String, i64)> {
    get(client, &format!("/applications/{application_id}/stages"))
        .await
        .as_array()
        .expect("an array")
        .iter()
        .map(|stage| {
            (
                stage["name"].as_str().expect("a name").to_owned(),
                stage["position"].as_i64().expect("a position"),
            )
        })
        .collect()
}

/// `(tool, has undo data)` of every logged action, in order.
async fn actions(pool: &PgPool) -> Vec<(i64, String, bool)> {
    sqlx::query_as("SELECT id, tool, undo_data IS NOT NULL FROM agent_actions ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read the actions")
}

/// The timeline line `source_email` should carry for a seeded email.
fn source(email: i64, seed: &Seed<'_>) -> Value {
    json!({"id": email, "subject": seed.subject, "from_name": seed.from_name,
           "from_address": seed.from_address, "sent_at": SENT_AT})
}

const HOOLI_INVITE: Seed<'static> = Seed {
    from_name: "Hooli Talent",
    from_address: "no-reply@hire.lever.co",
    subject: "Schedule your interview with Hooli",
    ats: Some("lever"),
    answer: Answer {
        class: "interview_invite",
        confidence: 0.9,
        company: Some("Hooli"),
        role: Some("Backend Engineer"),
        stage: Some("Onsite"),
    },
};

const VANDELAY_REJECTION: Seed<'static> = Seed {
    from_name: "Vandelay Industries",
    from_address: "no-reply@ashbyhq.com",
    subject: "Update on your application to Vandelay Industries",
    ats: Some("ashby"),
    answer: Answer {
        class: "rejection",
        confidence: 0.95,
        company: Some("Vandelay Industries"),
        role: Some("Senior Engineer"),
        stage: None,
    },
};

/// A low-confidence invite from Stark Industries.
const STARK_INVITE: Seed<'static> = Seed {
    from_name: "Stark Scheduling",
    from_address: "calendar@stark.example",
    subject: "Invitation: Interview with Stark Industries",
    ats: None,
    answer: Answer {
        class: "interview_invite",
        confidence: 0.6,
        company: Some("Stark Industries"),
        role: Some("Software Engineer"),
        stage: Some("Technical interview"),
    },
};

const GLOBEX_RECEIVED: Seed<'static> = Seed {
    from_name: "Globex",
    from_address: "no-reply@us.greenhouse-mail.io",
    subject: "Thank you for applying!",
    ats: Some("greenhouse"),
    answer: Answer {
        class: "other",
        confidence: 0.9,
        company: Some("Globex"),
        role: Some("Data Engineer"),
        stage: None,
    },
};

#[sqlx::test]
async fn confident_match_applies_with_undo(pool: PgPool) {
    let (client, _fake) = app(&pool).await;
    let hooli = seed_application(
        &pool,
        "Hooli",
        "Backend Engineer",
        "applied",
        &["Recruiter call"],
    )
    .await;
    let (email, classification) = seed(&pool, "hooli", HOOLI_INVITE).await;

    let output = run_match(&pool, classification).await;
    assert_eq!(
        output,
        json!({
            "email_id": email, "classification_id": classification, "status": "applied",
            "application_id": hooli, "confidence": 0.8,
            "reasons": ["company", "sender_name", "role"],
            "ops": [{"op": "set_status", "from": "applied", "to": "interviewing"},
                    {"op": "add_stage", "name": "Onsite"}],
            "review_item_id": null, "review_kind": null,
        })
    );

    let application = get(&client, &format!("/applications/{hooli}")).await;
    assert_eq!(application["status"], "interviewing");
    assert_eq!(
        stages_of(&client, hooli).await,
        [("Recruiter call".into(), 0), ("Onsite".into(), 1)]
    );
    let logged: Vec<(String, bool)> = actions(&pool)
        .await
        .into_iter()
        .map(|(_, tool, undo)| (tool, undo))
        .collect();
    assert_eq!(
        logged,
        [
            ("set_application_status".into(), true),
            ("add_application_stage".into(), true)
        ]
    );
    let events = get(&client, &format!("/applications/{hooli}/events")).await;
    let events = events.as_array().expect("an array");
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["kind"], "stage_added");
    assert_eq!(events[0]["stage_name"], "Onsite");
    assert_eq!(events[1]["kind"], "status_changed");
    assert_eq!(
        (&events[1]["from_status"], &events[1]["to_status"]),
        (&json!("applied"), &json!("interviewing"))
    );
    for event in events {
        assert_eq!(event["source_email"], source(email, &HOOLI_INVITE));
    }
    assert_eq!(
        match_row(&pool, classification).await,
        ("applied".into(), Some(hooli))
    );

    // A confident `other` email links the application and changes nothing.
    let initech = seed_application(&pool, "Initech", "Platform Engineer", "applied", &[]).await;
    let other = Seed {
        from_name: "Initech Recruiting",
        from_address: "recruiting@initech.example",
        subject: "Your application for Platform Engineer at Initech",
        ats: None,
        answer: Answer {
            class: "other",
            confidence: 0.9,
            company: Some("Initech"),
            role: Some("Platform Engineer"),
            stage: None,
        },
    };
    let (other_email, other_classification) = seed(&pool, "initech", other).await;
    let before = counts(&pool).await;
    let output = run_match(&pool, other_classification).await;
    assert_eq!(
        output,
        json!({
            "email_id": other_email, "classification_id": other_classification,
            "status": "no_action", "application_id": initech, "confidence": 1.0,
            "reasons": ["company", "sender_name", "sender_domain", "role"], "ops": [],
            "review_item_id": null, "review_kind": null,
        })
    );
    assert_eq!(counts(&pool).await, before);
    assert_eq!(status_of(&pool, initech).await, "applied");
    assert_eq!(
        match_row(&pool, other_classification).await,
        ("no_action".into(), Some(initech))
    );
}

#[sqlx::test]
async fn undo_restores_and_refuses_after_change(pool: PgPool) {
    let (client, _fake) = app(&pool).await;
    let hooli = seed_application(
        &pool,
        "Hooli",
        "Backend Engineer",
        "applied",
        &["Recruiter call"],
    )
    .await;
    let changed_at = |id: i64| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, DateTime<Utc>>(
                "SELECT status_changed_at FROM applications WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("read status_changed_at")
        }
    };
    let before = changed_at(hooli).await;
    let (_, classification) = seed(&pool, "hooli", HOOLI_INVITE).await;
    run_match(&pool, classification).await;
    // A stage the user adds after the email's.
    let (status, _) = post(
        &client,
        &format!("/applications/{hooli}/stages"),
        Some(json!({"name": "Team match"})),
    )
    .await;
    assert_eq!(status, Status::Created);
    let logged = actions(&pool).await;
    let (status_action, stage_action) = (logged[0].0, logged[1].0);

    let (status, undone) = post(&client, &format!("/actions/{status_action}/undo"), None).await;
    assert_eq!(status, Status::Ok, "{undone}");
    assert_eq!(status_of(&pool, hooli).await, "applied");
    assert_eq!(changed_at(hooli).await, before);
    let sourced = "SELECT count(*) FROM application_events WHERE source_email_id IS NOT NULL";
    assert_eq!(
        count(&pool, sourced).await,
        1,
        "only the stage event is left"
    );

    let (status, undone) = post(&client, &format!("/actions/{stage_action}/undo"), None).await;
    assert_eq!(status, Status::Ok, "{undone}");
    assert_eq!(
        stages_of(&client, hooli).await,
        [("Recruiter call".into(), 0), ("Team match".into(), 1)]
    );
    assert_eq!(count(&pool, sourced).await, 0);

    // A status the user changed since can't be undone.
    let vandelay = seed_application(
        &pool,
        "Vandelay Industries",
        "Senior Engineer",
        "applied",
        &[],
    )
    .await;
    let (_, rejection) = seed(&pool, "vandelay", VANDELAY_REJECTION).await;
    run_match(&pool, rejection).await;
    let rejected_action = actions(&pool).await.last().expect("an action").0;
    let path = format!("/applications/{vandelay}/status");
    assert_eq!(
        put(&client, &path, json!({"status": "interviewing"})).await,
        Status::Ok
    );
    let undo = format!("/actions/{rejected_action}/undo");
    let before = counts(&pool).await;
    assert_eq!(post(&client, &undo, None).await.0, Status::Conflict);
    assert_eq!(status_of(&pool, vandelay).await, "interviewing");
    assert_eq!(counts(&pool).await, before);
    // Nor one changed back to the email's status by a later event.
    assert_eq!(
        put(&client, &path, json!({"status": "rejected"})).await,
        Status::Ok
    );
    let before = counts(&pool).await;
    assert_eq!(post(&client, &undo, None).await.0, Status::Conflict);
    assert_eq!(status_of(&pool, vandelay).await, "rejected");
    assert_eq!(counts(&pool).await, before);

    // A completed stage can't be undone.
    let stark = seed_application(
        &pool,
        "Stark Industries",
        "Software Engineer",
        "applied",
        &[],
    )
    .await;
    let confident = Seed {
        answer: Answer {
            confidence: 0.95,
            ..STARK_INVITE.answer
        },
        ..STARK_INVITE
    };
    let (_, invite) = seed(&pool, "stark", confident).await;
    run_match(&pool, invite).await;
    let stage_action = actions(&pool).await.last().expect("an action").0;
    let stage = &get(&client, &format!("/applications/{stark}/stages")).await[0];
    let stage_path = format!("/applications/{stark}/stages/{}", stage["id"]);
    assert_eq!(
        put(&client, &stage_path, json!({"completed": true})).await,
        Status::Ok
    );
    let before = counts(&pool).await;
    let undo = format!("/actions/{stage_action}/undo");
    assert_eq!(post(&client, &undo, None).await.0, Status::Conflict);
    assert_eq!(
        stages_of(&client, stark).await,
        [("Technical interview".into(), 0)]
    );
    assert_eq!(counts(&pool).await, before);
}

/// `(kind, confidence, payload)` of review item `id`.
async fn review_item(pool: &PgPool, id: &Value) -> (String, f64, Value) {
    sqlx::query_as("SELECT kind, confidence, payload FROM review_items WHERE id = $1")
        .bind(id.as_i64().expect("a review item id"))
        .fetch_one(pool)
        .await
        .expect("read the review item")
}

fn email_json(seed: &Seed<'_>) -> Value {
    json!({"subject": seed.subject, "from_name": seed.from_name,
           "from_address": seed.from_address, "sent_at": SENT_AT})
}

#[sqlx::test]
async fn unsure_matches_go_to_review(pool: PgPool) {
    let (_client, _fake) = app(&pool).await;

    // A linked match the classifier wasn't sure of.
    let stark = seed_application(
        &pool,
        "Stark Industries",
        "Software Engineer",
        "applied",
        &[],
    )
    .await;
    let (email, classification) = seed(&pool, "stark", STARK_INVITE).await;
    let output = run_match(&pool, classification).await;
    assert_eq!(output["status"], "review", "{output}");
    assert_eq!(output["review_kind"], "email.match");
    assert_eq!(output["application_id"], json!(stark));
    let (kind, confidence, payload) = review_item(&pool, &output["review_item_id"]).await;
    assert_eq!((kind.as_str(), confidence), ("email.match", 0.6));
    assert_eq!(
        payload,
        json!({
            "email_id": email, "classification_id": classification,
            "class": "interview_invite", "email": email_json(&STARK_INVITE),
            "application_id": stark, "company": "Stark Industries",
            "title": "Software Engineer",
            "ops": [{"op": "set_status", "from": "applied", "to": "interviewing"},
                    {"op": "add_stage", "name": "Technical interview"}],
            "reasons": ["company", "role"], "review_reasons": ["low_confidence"],
            "candidates": [{"application_id": stark, "company": "Stark Industries",
                            "title": "Software Engineer", "status": "applied", "score": 0.8,
                            "reasons": ["company", "role"]}],
        })
    );
    assert_eq!(match_row(&pool, classification).await.0, "review");

    // Two applications at one company, and no role to tell them apart.
    let backend = seed_application(&pool, "Pied Piper", "Backend Engineer", "applied", &[]).await;
    let data = seed_application(&pool, "Pied Piper", "Data Engineer", "applied", &[]).await;
    let ambiguous = Seed {
        from_name: "Pied Piper Recruiting",
        from_address: "jobs@piedpiper.example",
        subject: "Your application",
        ats: None,
        answer: Answer {
            class: "rejection",
            confidence: 0.95,
            company: Some("Pied Piper"),
            role: None,
            stage: None,
        },
    };
    let (_, classification) = seed(&pool, "pied", ambiguous).await;
    let output = run_match(&pool, classification).await;
    assert_eq!(output["application_id"], Value::Null, "{output}");
    let (_, confidence, payload) = review_item(&pool, &output["review_item_id"]).await;
    assert_eq!(
        confidence, 0.8,
        "the top score, below the classifier's 0.95"
    );
    assert_eq!(payload["review_reasons"], json!(["ambiguous"]));
    let candidates: Vec<&Value> = payload["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .map(|candidate| &candidate["application_id"])
        .collect();
    assert_eq!(candidates, [&json!(backend), &json!(data)]);

    // A linked, confident offer for a rejected application.
    let wayne = seed_application(
        &pool,
        "Wayne Enterprises",
        "Staff Engineer",
        "rejected",
        &[],
    )
    .await;
    let offer = Seed {
        from_name: "Wayne Enterprises People Team",
        from_address: "people@wayne.example",
        subject: "Your offer from Wayne Enterprises",
        ats: None,
        answer: Answer {
            class: "offer",
            confidence: 0.95,
            company: Some("Wayne Enterprises"),
            role: Some("Staff Engineer"),
            stage: None,
        },
    };
    let (_, classification) = seed(&pool, "wayne", offer).await;
    let output = run_match(&pool, classification).await;
    assert_eq!(output["application_id"], json!(wayne), "{output}");
    let (_, confidence, payload) = review_item(&pool, &output["review_item_id"]).await;
    assert_eq!(confidence, 0.8);
    assert_eq!(payload["review_reasons"], json!(["unexpected_status"]));
    assert_eq!(
        payload["ops"],
        json!([{"op": "set_status", "from": "rejected", "to": "offer"}])
    );

    // Nothing changed.
    for (id, status) in [
        (stark, "applied"),
        (backend, "applied"),
        (data, "applied"),
        (wayne, "rejected"),
    ] {
        assert_eq!(status_of(&pool, id).await, status);
    }
    assert_eq!(counts(&pool).await, (0, 0));

    // An email with a company but no matching application proposes one.
    let (email, classification) = seed(&pool, "globex", GLOBEX_RECEIVED).await;
    let output = run_match(&pool, classification).await;
    assert_eq!(
        (
            &output["status"],
            &output["review_kind"],
            &output["application_id"]
        ),
        (
            &json!("review"),
            &json!("email.new_application"),
            &Value::Null
        ),
        "{output}"
    );
    let (kind, confidence, payload) = review_item(&pool, &output["review_item_id"]).await;
    assert_eq!((kind.as_str(), confidence), ("email.new_application", 0.9));
    assert_eq!(
        payload,
        json!({
            "email_id": email, "classification_id": classification, "class": "other",
            "email": email_json(&GLOBEX_RECEIVED),
            "application": {"company": "Globex", "title": "Data Engineer", "status": "applied"},
        })
    );

    // One without a company stores nothing.
    let personal = Seed {
        from_name: "Pat Lin",
        from_address: "pat@friends.example",
        subject: "Dinner?",
        ats: None,
        answer: Answer {
            class: "other",
            confidence: 0.9,
            company: None,
            role: None,
            stage: None,
        },
    };
    let (_, classification) = seed(&pool, "pat", personal).await;
    let output = run_match(&pool, classification).await;
    assert_eq!(output["status"], "unmatched", "{output}");
    assert_eq!(output["review_item_id"], Value::Null);
    assert_eq!(count(&pool, "SELECT count(*) FROM review_items").await, 4);
    assert_eq!(counts(&pool).await, (0, 0));
}

#[sqlx::test]
async fn reruns_and_superseded_classifications_apply_once(pool: PgPool) {
    let (_client, _fake) = app(&pool).await;
    let vandelay = seed_application(
        &pool,
        "Vandelay Industries",
        "Senior Engineer",
        "applied",
        &[],
    )
    .await;
    let (email, first) = seed(&pool, "vandelay", VANDELAY_REJECTION).await;
    let output = run_match(&pool, first).await;
    assert_eq!(output["status"], "applied", "{output}");
    let applied = counts(&pool).await;
    assert_eq!(applied, (1, 1));

    // A second run of a finished match returns its stored outcome.
    assert_eq!(run_match(&pool, first).await, output);
    assert_eq!(counts(&pool).await, applied);

    // A crash after the tool call but before the match row was written: the rerun plans
    // again from the current state and finds nothing left to do.
    sqlx::query("UPDATE email_matches SET status = 'pending' WHERE classification_id = $1")
        .bind(first)
        .execute(&pool)
        .await
        .expect("reset the match row");
    let again = run_match(&pool, first).await;
    assert_eq!(
        (&again["status"], &again["application_id"]),
        (&json!("no_action"), &json!(vandelay)),
        "{again}"
    );
    assert_eq!(counts(&pool).await, applied);
    assert_eq!(status_of(&pool, vandelay).await, "rejected");

    // A newer classification of the email makes the older one stale.
    let newer = classify(&pool, email, "key-newer", VANDELAY_REJECTION.answer).await;
    sqlx::query("UPDATE email_matches SET status = 'pending' WHERE classification_id = $1")
        .bind(first)
        .execute(&pool)
        .await
        .expect("reset the match row");
    let skipped = json!({
        "email_id": email, "classification_id": first, "status": "skipped",
        "application_id": null, "confidence": null, "reasons": [], "ops": [],
        "review_item_id": null, "review_kind": null,
    });
    assert_eq!(run_match(&pool, first).await, skipped);
    assert_eq!(
        match_row(&pool, first).await,
        ("pending".into(), Some(vandelay)),
        "a skipped run leaves the row as it was"
    );

    // A superseded classification, and one that is gone, are skipped too.
    sqlx::query("UPDATE email_classifications SET status = 'superseded' WHERE id = $1")
        .bind(newer)
        .execute(&pool)
        .await
        .expect("supersede the classification");
    let output = run_match(&pool, newer).await;
    assert_eq!(output["status"], "skipped", "{output}");
    let output = run_match(&pool, i64::MAX).await;
    assert_eq!(
        (&output["status"], &output["email_id"]),
        (&json!("skipped"), &Value::Null)
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM email_matches").await,
        1,
        "a skipped run stores no row"
    );
    assert_eq!(counts(&pool).await, applied);
    assert_eq!(status_of(&pool, vandelay).await, "rejected");
}

/// `(status, resolved_at is set)` of review item `id`.
async fn item_state(pool: &PgPool, id: i64) -> (String, bool) {
    sqlx::query_as("SELECT status, resolved_at IS NOT NULL FROM review_items WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read the review item")
}

#[sqlx::test]
async fn accept_applies_email_review_items(pool: PgPool) {
    let (client, _fake) = app(&pool).await;
    let stark = seed_application(
        &pool,
        "Stark Industries",
        "Software Engineer",
        "applied",
        &[],
    )
    .await;
    let (email, classification) = seed(&pool, "stark", STARK_INVITE).await;
    let output = run_match(&pool, classification).await;
    let item = output["review_item_id"].as_i64().expect("a review item");
    let accept = format!("/review-items/{item}/accept");

    let (status, body) = post(&client, &accept, Some(json!({}))).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(body["status"], "accepted");
    assert!(body["resolved_at"].is_string(), "{body}");
    assert_eq!(status_of(&pool, stark).await, "interviewing");
    assert_eq!(
        stages_of(&client, stark).await,
        [("Technical interview".into(), 0)]
    );
    let events = get(&client, &format!("/applications/{stark}/events")).await;
    let kinds: Vec<&Value> = events
        .as_array()
        .expect("an array")
        .iter()
        .map(|event| &event["kind"])
        .collect();
    assert_eq!(kinds, [&json!("stage_added"), &json!("status_changed")]);
    for event in events.as_array().expect("an array") {
        assert_eq!(event["source_email"], source(email, &STARK_INVITE));
    }
    assert_eq!(
        match_row(&pool, classification).await,
        ("applied".into(), Some(stark))
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM agent_actions").await,
        0,
        "an accept is the user's own change"
    );

    // A second accept is a conflict.
    let before = counts(&pool).await;
    let (status, _) = post(&client, &accept, Some(json!({}))).await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(counts(&pool).await, before);

    // A proposed application is created.
    let (email, classification) = seed(&pool, "globex", GLOBEX_RECEIVED).await;
    let output = run_match(&pool, classification).await;
    let item = output["review_item_id"].as_i64().expect("a review item");
    let (status, body) = post(
        &client,
        &format!("/review-items/{item}/accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let (globex, company, title, app_status, source_column): (i64, String, String, String, String) =
        sqlx::query_as(
            "SELECT id, company, title, status, source FROM applications WHERE company = 'Globex'",
        )
        .fetch_one(&pool)
        .await
        .expect("the created application");
    assert_eq!(
        (
            company.as_str(),
            title.as_str(),
            app_status.as_str(),
            source_column.as_str()
        ),
        ("Globex", "Data Engineer", "applied", "email")
    );
    let events = get(&client, &format!("/applications/{globex}/events")).await;
    assert_eq!(events.as_array().expect("an array").len(), 1);
    assert_eq!(events[0]["kind"], "created");
    assert_eq!(events[0]["to_status"], "applied");
    assert_eq!(events[0]["source_email"], source(email, &GLOBEX_RECEIVED));
    assert_eq!(
        match_row(&pool, classification).await,
        ("applied".into(), Some(globex))
    );

    // Other kinds can't be accepted.
    let tidy: i64 = sqlx::query_scalar(
        "INSERT INTO review_items (run_id, run_step_id, kind, confidence, payload) \
         SELECT run_id, id, 'tidy.note', 0.5, '{\"application_id\": 1, \"note\": \"n\"}' \
         FROM run_steps ORDER BY id LIMIT 1 RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert a tidy.note item");
    let (status, _) = post(
        &client,
        &format!("/review-items/{tidy}/accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(item_state(&pool, tidy).await, ("pending".into(), false));

    // An application whose status changed since is a conflict, with nothing changed.
    let wayne =
        seed_application(&pool, "Wayne Enterprises", "Staff Engineer", "applied", &[]).await;
    let unsure = Seed {
        from_name: "Wayne Enterprises People Team",
        from_address: "people@wayne.example",
        subject: "Your offer from Wayne Enterprises",
        ats: None,
        answer: Answer {
            class: "offer",
            confidence: 0.6,
            company: Some("Wayne Enterprises"),
            role: Some("Staff Engineer"),
            stage: None,
        },
    };
    let (_, classification) = seed(&pool, "wayne", unsure).await;
    let output = run_match(&pool, classification).await;
    let item = output["review_item_id"].as_i64().expect("a review item");
    assert_eq!(
        put(
            &client,
            &format!("/applications/{wayne}/status"),
            json!({"status": "interviewing"})
        )
        .await,
        Status::Ok
    );
    let before = counts(&pool).await;
    let (status, _) = post(
        &client,
        &format!("/review-items/{item}/accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(status_of(&pool, wayne).await, "interviewing");
    assert_eq!(item_state(&pool, item).await, ("pending".into(), false));
    assert_eq!(match_row(&pool, classification).await.0, "review");
    assert_eq!(counts(&pool).await, before);

    let (status, _) = post(
        &client,
        &format!("/review-items/{}/accept", i64::MAX),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::NotFound);
}

#[sqlx::test]
async fn accept_refuses_cross_site_and_form_posts(pool: PgPool) {
    let (client, _fake) = app(&pool).await;
    let stark = seed_application(
        &pool,
        "Stark Industries",
        "Software Engineer",
        "applied",
        &[],
    )
    .await;
    let (_, classification) = seed(&pool, "stark", STARK_INVITE).await;
    let output = run_match(&pool, classification).await;
    let item = output["review_item_id"].as_i64().expect("a review item");
    let path = format!("/review-items/{item}/accept");

    let response = client
        .post(path.clone())
        .header(host())
        .header(ContentType::JSON)
        .header(Header::new("Sec-Fetch-Site", "cross-site"))
        .body("{}")
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Forbidden, "cross-site");
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
        assert_eq!(response.status(), Status::Forbidden, "{form} {body}");
    }

    assert_eq!(item_state(&pool, item).await, ("pending".into(), false));
    assert_eq!(status_of(&pool, stark).await, "applied");
    assert_eq!(counts(&pool).await, (0, 0));
}

/// Stores an email from `from_address` (no classification) and returns its id and current
/// input key.
async fn seed_unclassified(pool: &PgPool, provider_id: &str, from_address: &str) -> (i64, String) {
    let thread: i64 = sqlx::query_scalar(
        "INSERT INTO email_threads (subject, subject_key) VALUES ('Hello', 'hello') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert a thread");
    let sent_at = instant(SENT_AT);
    let headers = json!([{"name": "Date", "value": sent_at.to_rfc2822()}]);
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO emails (source, provider_message_id, thread_id, from_address, subject, \
         sent_at, headers, raw, body_text, body_source, verdict, verdict_reasons, \
         normalizer_version, heuristics_version) \
         VALUES ('fixture', $1, $2, $3, 'Hello', $4, $5, '\\x00', 'Hi', 'plain', 'candidate', \
         '[]', 1, 1) RETURNING id",
    )
    .bind(provider_id)
    .bind(thread)
    .bind(from_address)
    .bind(sent_at)
    .bind(headers)
    .fetch_one(pool)
    .await
    .expect("insert an email");
    let key = classify_email::input_key(&classify_email::KeyInput {
        subject: "Hello",
        from_address: Some(from_address),
        from_name: None,
        sent_at: Some(sent_at),
        body_text: "Hi",
        signature: None,
        ats: None,
    });
    (id, key)
}

/// Starts a `classify_email` run with `input` and returns its id.
async fn start_classify(pool: &PgPool, input: Value) -> i64 {
    runs::start(
        pool,
        &EventBus::default(),
        &Wake::default(),
        &workflows::runtime(),
        classify_email::WORKFLOW,
        input,
    )
    .await
    .expect("start a run")
    .id
}

/// `(id, status, input)` of every `match_email` run, oldest first.
async fn match_runs(pool: &PgPool) -> Vec<(i64, String, Value)> {
    sqlx::query_as(
        "SELECT id, status, input FROM agent_runs WHERE workflow = 'match_email' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("read the match runs")
}

/// `(classification_id, run_id)` of every match row, oldest first.
async fn match_rows(pool: &PgPool) -> Vec<(i64, Option<i64>)> {
    sqlx::query_as("SELECT classification_id, run_id FROM email_matches ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read the match rows")
}

#[sqlx::test]
async fn classification_queues_one_match_run(pool: PgPool) {
    let (_client, _fake) = app_with(&pool, "classify_by_email").await;

    // A run ingest queued (with its key) queues one match run for its classification.
    let (email, key) = seed_unclassified(&pool, "dana", "dana@soylent.example").await;
    let input = json!({"email_id": email, "input_key": key});
    let classified = until(
        &pool,
        start_classify(&pool, input.clone()).await,
        "succeeded",
    )
    .await;
    let classification = classified["classification_id"]
        .as_i64()
        .expect("a classification");
    let runs = match_runs(&pool).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].2, json!({"classification_id": classification}));
    assert_eq!(match_rows(&pool).await, [(classification, Some(runs[0].0))]);
    // That run matches it: no company, so nothing to do.
    let output = until(&pool, runs[0].0, "succeeded").await;
    assert_eq!(output["status"], "unmatched", "{output}");

    // Rerunning the classify run (the "already classified" path) queues no second one.
    until(&pool, start_classify(&pool, input).await, "succeeded").await;
    assert_eq!(match_runs(&pool).await.len(), 1);

    // A run without a key (an eval case or a manual start) queues none.
    let (manual, _) = seed_unclassified(&pool, "manual", "jo@initech.example").await;
    until(
        &pool,
        start_classify(&pool, json!({"email_id": manual})).await,
        "succeeded",
    )
    .await;
    assert_eq!(match_runs(&pool).await.len(), 1);

    // Nor does an invalid answer.
    let (invalid, key) = seed_unclassified(&pool, "invalid", "invalid-answer@test.example").await;
    let run = start_classify(&pool, json!({"email_id": invalid, "input_key": key})).await;
    until(&pool, run, "failed").await;
    assert_eq!(match_runs(&pool).await.len(), 1);
    assert_eq!(match_rows(&pool).await.len(), 1);

    // With the kill switch turned on during the model call, the match run is stored paused.
    let (slow, key) = seed_unclassified(&pool, "slow", "slow-answer@test.example").await;
    let run = start_classify(&pool, json!({"email_id": slow, "input_key": key})).await;
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let running: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM run_steps WHERE run_id = $1 AND status = 'running')",
        )
        .bind(run)
        .fetch_one(&pool)
        .await
        .expect("read the step");
        if running {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "the step never ran");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sqlx::query("UPDATE kill_switch SET paused = true, updated_at = now() WHERE id")
        .execute(&pool)
        .await
        .expect("turn the kill switch on");
    until(&pool, run, "succeeded").await;
    let runs = match_runs(&pool).await;
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(runs[1].1, "paused");
}

/// Waits until no run is queued or running.
async fn settled(pool: &PgPool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let busy = count(
            pool,
            "SELECT count(*) FROM agent_runs WHERE status IN ('queued', 'running')",
        )
        .await;
        if busy == 0 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{busy} run(s) never settled"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn create_application(client: &Client, company: &str, title: &str) -> i64 {
    let (status, body) = post(
        client,
        "/applications",
        Some(json!({"company": company, "title": title, "status": "applied"})),
    )
    .await;
    assert_eq!(status, Status::Created, "{body}");
    body["id"].as_i64().expect("an id")
}

#[sqlx::test]
async fn fixture_import_flows_to_application_updates(pool: PgPool) {
    let (client, _fake) = app_with(&pool, "classify_by_email").await;
    let vandelay = create_application(&client, "Vandelay Industries", "Senior Engineer").await;
    let hooli = create_application(&client, "Hooli", "Backend Engineer").await;
    let stark = create_application(&client, "Stark Industries", "Software Engineer").await;

    let response = client
        .post("/dev/emails/import-fixtures")
        .header(host())
        .header(Header::new("X-Waypoint-Dev", "1"))
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok);
    settled(&pool).await;

    let unsettled = count(
        &pool,
        "SELECT count(*) FROM agent_runs WHERE status <> 'succeeded'",
    )
    .await;
    assert_eq!(unsettled, 0, "every classify and match run succeeded");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'match_email'"
        )
        .await,
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'classify_email'"
        )
        .await,
        "one match run per classification"
    );

    assert_eq!(status_of(&pool, vandelay).await, "rejected");
    assert_eq!(status_of(&pool, hooli).await, "interviewing");
    assert_eq!(
        stages_of(&client, hooli).await,
        [("Technical interview".into(), 0)]
    );
    assert_eq!(status_of(&pool, stark).await, "applied");

    let logged = actions(&pool).await;
    assert_eq!(logged.len(), 3, "{logged:?}");
    assert!(logged.iter().all(|(_, _, undo)| *undo), "{logged:?}");
    let items: Vec<(String, Value)> = sqlx::query_as(
        "SELECT kind, payload FROM review_items WHERE status = 'pending' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("read the review items");
    let mut kinds: Vec<&str> = items.iter().map(|(kind, _)| kind.as_str()).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["email.match", "email.new_application"]);
    for (kind, payload) in &items {
        if kind == "email.match" {
            assert_eq!(payload["application_id"], json!(stark), "{payload}");
            assert_eq!(payload["review_reasons"], json!(["low_confidence"]));
        } else {
            assert_eq!(
                payload["application"],
                json!({"company": "Globex", "title": "Data Engineer", "status": "applied"})
            );
        }
    }

    let events = get(&client, &format!("/applications/{vandelay}/events")).await;
    let changed = events
        .as_array()
        .expect("an array")
        .iter()
        .find(|event| event["kind"] == "status_changed")
        .expect("a status change");
    let email: i64 =
        sqlx::query_scalar("SELECT id FROM emails WHERE provider_message_id = 'ashby-rejection'")
            .fetch_one(&pool)
            .await
            .expect("the rejection email");
    assert_eq!(
        changed["source_email"],
        json!({"id": email, "subject": "Update on your application to Vandelay Industries",
               "from_name": "Vandelay Industries", "from_address": "no-reply@ashbyhq.com",
               "sent_at": "2026-09-10T21:45:00Z"})
    );

    // Undoing the rejection over HTTP restores the application.
    let action: i64 = sqlx::query_scalar(
        "SELECT id FROM agent_actions WHERE tool = 'set_application_status' \
         AND (undo_data ->> 'application_id')::bigint = $1",
    )
    .bind(vandelay)
    .fetch_one(&pool)
    .await
    .expect("the rejection's action");
    let (status, body) = post(&client, &format!("/actions/{action}/undo"), None).await;
    assert_eq!(status, Status::Ok, "{body}");
    assert_eq!(status_of(&pool, vandelay).await, "applied");
}
