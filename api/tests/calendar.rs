//! Calendar interview proposals end to end, against a fake Google (wiremock): a linked
//! interview email parks one `propose_interview` run per future slot at the approval gate,
//! approving one creates the Google Calendar event once and links it to the application,
//! rejecting one never calls Google, and Calendar failures end the proposal. No model is
//! called.

mod common;

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::crypto::{self, TokenKey};
use waypoint_api::events::EventBus;
use waypoint_api::google;
use waypoint_api::jobs::Wake;
use waypoint_api::runs;
use waypoint_api::workflows::{self, match_email};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TIMEOUT: Duration = Duration::from_secs(20);
const KEY: [u8; 32] = [7; 32];
const EVENTS: &str = "/calendar/v3/calendars/primary/events";
const SUBJECT: &str = "Interview availability";
/// Every seeded email's `sent_at`.
const SENT_AT: &str = "2026-10-20T16:00:00Z";
const LINK: &str = "https://www.google.com/calendar/event?eid=evt-1";

/// A fake token, built at runtime so it never looks like a committed secret.
fn fake(kind: &str) -> String {
    format!("fake-{kind}-{}", "for-tests")
}

fn google_config(server: &MockServer) -> google::Config {
    google::Config {
        client_id: Some("test-client.apps.googleusercontent.com".into()),
        client_secret: Some(fake("client-secret")),
        token_key: Some(TokenKey::new(KEY)),
        auth_url: format!("{}/auth", server.uri()),
        token_url: format!("{}/token", server.uri()),
        revoke_url: format!("{}/revoke", server.uri()),
        redirect_uri: google::config::redirect_uri(8000),
        gmail: waypoint_api::gmail::Config {
            base_url: server.uri(),
            ..Default::default()
        },
        calendar: waypoint_api::calendar::Config {
            base_url: server.uri(),
        },
    }
}

/// Stores a connected account whose access token is `fake("access")` (valid for an hour),
/// and a Gmail sync that just finished, so the worker's startup catch-up does nothing.
async fn seed_account(pool: &PgPool) {
    let key = TokenKey::new(KEY);
    sqlx::query(
        "INSERT INTO google_accounts \
             (refresh_token, access_token, access_token_expires_at, scopes, status) \
         VALUES ($1, $2, now() + interval '1 hour', $3, 'connected')",
    )
    .bind(crypto::seal(
        &key,
        google::REFRESH_TOKEN_AAD,
        fake("refresh").as_bytes(),
    ))
    .bind(crypto::seal(
        &key,
        google::ACCESS_TOKEN_AAD,
        fake("access").as_bytes(),
    ))
    .bind(google::SCOPES.map(String::from).to_vec())
    .execute(pool)
    .await
    .expect("seed the account");
    sqlx::query("INSERT INTO gmail_sync_state (id, last_finished_at) VALUES (true, now())")
        .execute(pool)
        .await
        .expect("seed the sync state");
}

/// The API on a fake Google, with a runner that would hang on any model call. Keep the
/// `Fake` alive as long as the client.
async fn app(pool: &PgPool, server: &MockServer) -> (Client, common::Fake) {
    seed_account(pool).await;
    let fake = common::fake("hang", Duration::from_secs(30));
    let rocket = common::app_with_google(
        pool.clone(),
        fake.runner.clone(),
        common::fast_config(),
        google_config(server),
    );
    (Client::tracked(rocket).await.expect("valid rocket"), fake)
}

/// Calendar answers every insert with event `evt-1`.
async fn calendar_ok(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path(EVENTS))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": "evt-1", "htmlLink": LINK})),
        )
        .mount(server)
        .await;
}

/// The requests the fake Google received at `endpoint`.
async fn received(server: &MockServer, endpoint: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("recording is on")
        .into_iter()
        .filter(|request| request.url.path() == endpoint)
        .collect()
}

/// `(status, JSON body or null)`, sent from the app's own origin.
async fn post(client: &Client, path: &str, body: Option<Value>) -> (Status, Value) {
    let mut request = client
        .post(path.to_owned())
        .header(Header::new("Host", common::HOST))
        .header(Header::new("Sec-Fetch-Site", "same-origin"));
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
    let response = client
        .get(path.to_owned())
        .header(Header::new("Host", common::HOST))
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok, "GET {path}");
    response.into_json().await.expect("a JSON body")
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"))
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant")
        .with_timezone(&Utc)
}

/// A normalized proposed time, as `classify_email` stores it.
fn time(text: &str, zone: Option<&str>, start: Option<&str>, end: Option<&str>) -> Value {
    json!({"text": text, "start_local": "2030-11-03T14:00:00", "end_local": null,
           "time_zone": zone, "start_utc": start, "end_utc": end})
}

/// The two future New York times most tests propose.
fn two_slots() -> Value {
    json!([
        time(
            "Tue Nov 3, 2pm ET",
            Some("America/New_York"),
            Some("2030-11-03T19:00:00Z"),
            Some("2030-11-03T19:45:00Z")
        ),
        time(
            "Wed Nov 4, 10am ET",
            Some("America/New_York"),
            Some("2030-11-04T15:00:00Z"),
            None
        ),
    ])
}

/// Inserts an `applied` application.
async fn seed_application(pool: &PgPool, company: &str, title: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO applications (company, title, status) VALUES ($1, $2, 'applied') \
         RETURNING id",
    )
    .bind(company)
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("insert an application")
}

/// What a seeded email from `company` says.
struct Email<'a> {
    company: &'a str,
    role: &'a str,
    class: &'a str,
    confidence: f64,
    times: Value,
}

/// Stores an email from `<company> Recruiting` in a thread of its own, with a `classified`
/// row (stage `Technical interview`, interviewer `Gavin Belson`). Returns
/// `(email_id, classification_id)`.
async fn seed_email(pool: &PgPool, provider_id: &str, email: &Email<'_>) -> (i64, i64) {
    let thread: i64 = sqlx::query_scalar(
        "INSERT INTO email_threads (subject, subject_key) VALUES ($1, lower($1)) RETURNING id",
    )
    .bind(SUBJECT)
    .fetch_one(pool)
    .await
    .expect("insert a thread");
    let email_id: i64 = sqlx::query_scalar(
        "INSERT INTO emails (source, provider_message_id, thread_id, from_address, from_name, \
         subject, sent_at, headers, raw, body_text, body_source, verdict, verdict_reasons, \
         normalizer_version, heuristics_version) \
         VALUES ('fixture', $1, $2, $3, $4, $5, $6, '[]', '\\x00', 'Hi', 'plain', 'candidate', \
         '[]', 1, 1) RETURNING id",
    )
    .bind(provider_id)
    .bind(thread)
    .bind(format!(
        "recruiting@{}.example",
        email.company.to_lowercase()
    ))
    .bind(format!("{} Recruiting", email.company))
    .bind(SUBJECT)
    .bind(instant(SENT_AT))
    .fetch_one(pool)
    .await
    .expect("insert an email");
    let classification: i64 = sqlx::query_scalar(
        "INSERT INTO email_classifications (email_id, input_key, classifier_version, status, \
         class, confidence, company, role, stage, interviewer, proposed_times) \
         VALUES ($1, $2, 1, 'classified', $3, $4, $5, $6, 'Technical interview', \
         'Gavin Belson', $7) RETURNING id",
    )
    .bind(email_id)
    .bind(format!("key-{provider_id}"))
    .bind(email.class)
    .bind(email.confidence)
    .bind(email.company)
    .bind(email.role)
    .bind(&email.times)
    .fetch_one(pool)
    .await
    .expect("insert a classification");
    (email_id, classification)
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

/// Runs `match_email` for `classification_id` and returns its output.
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

/// One `interview_events` row.
#[derive(Debug, sqlx::FromRow)]
struct Proposal {
    id: i64,
    status: String,
    run_id: Option<i64>,
    request: Value,
    google_event_id: Option<String>,
    html_link: Option<String>,
    start_at: Option<DateTime<Utc>>,
    end_at: Option<DateTime<Utc>>,
    error: Option<String>,
}

impl Proposal {
    fn run(&self) -> i64 {
        self.run_id.expect("the proposal has a run")
    }
}

/// Application `application_id`'s proposals, by proposed start.
async fn proposals(pool: &PgPool, application_id: i64) -> Vec<Proposal> {
    sqlx::query_as(
        "SELECT id, status, run_id, request, google_event_id, html_link, start_at, end_at, \
         error FROM interview_events WHERE application_id = $1 ORDER BY proposed_start",
    )
    .bind(application_id)
    .fetch_all(pool)
    .await
    .expect("read the proposals")
}

/// `(id, tool, status, input)` of run `run_id`'s pending actions.
async fn actions(pool: &PgPool, run_id: i64) -> Vec<(i64, String, String, Value)> {
    sqlx::query_as(
        "SELECT id, tool, status, input FROM pending_actions WHERE run_id = $1 ORDER BY id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read the pending actions")
}

/// The one pending action of `proposal`'s run, once it is parked.
async fn parked(pool: &PgPool, proposal: &Proposal) -> i64 {
    until(pool, proposal.run(), "awaiting_approval").await;
    let found = actions(pool, proposal.run()).await;
    assert_eq!(found.len(), 1, "{found:?}");
    let (id, tool, status, input) = &found[0];
    assert_eq!(tool, "create_calendar_event");
    assert_eq!(status, "pending");
    assert_eq!(input, &proposal.request);
    *id
}

async fn action_status(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM pending_actions WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read the action")
}

/// The request stored for the `index`th of `count` New York slots of a Hooli email.
fn hooli_request(index: usize, count: usize, text: &str, start: &str, end: &str) -> Value {
    json!({
        "summary": "Interview: Hooli — Technical interview",
        "description": format!(
            "Role: Backend Engineer\nStage: Technical interview\nInterviewer: Gavin Belson\n\
             Proposed time {index} of {count}: \"{text}\"\n\
             From email: \"{SUBJECT}\" · Hooli Recruiting\nAdded by Waypoint after your approval."
        ),
        "start": start,
        "end": end,
        "time_zone": "America/New_York",
    })
}

/// An interview email from Hooli, confident enough to apply.
fn hooli_email(times: Value) -> Email<'static> {
    Email {
        company: "Hooli",
        role: "Backend Engineer",
        class: "interview_invite",
        confidence: 0.9,
        times,
    }
}

#[sqlx::test]
async fn linked_invites_park_one_proposal_per_future_slot(pool: PgPool) {
    let server = MockServer::start().await;
    calendar_ok(&server).await;
    let (client, _fake) = app(&pool, &server).await;

    // Past, zone-less, repeated and future times: two future slots.
    let hooli = seed_application(&pool, "Hooli", "Backend Engineer").await;
    let times = json!([
        time("last year", Some("UTC"), Some("2020-01-07T10:00:00Z"), None),
        time("Tuesday 2pm", None, None, None),
        time(
            "Tue Nov 3, 2pm ET",
            Some("America/New_York"),
            Some("2030-11-03T19:00:00Z"),
            Some("2030-11-03T19:45:00Z")
        ),
        time("again", Some("UTC"), Some("2030-11-03T19:00:00Z"), None),
        time(
            "Wed Nov 4, 10am ET",
            Some("America/New_York"),
            Some("2030-11-04T15:00:00Z"),
            None
        ),
    ]);
    let (_, classification) = seed_email(&pool, "hooli-1", &hooli_email(times)).await;
    let output = run_match(&pool, classification).await;
    assert_eq!(output["status"], "applied", "{output}");
    assert_eq!(output["application_id"], hooli);

    let stored = proposals(&pool, hooli).await;
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert_eq!(
        stored[0].request,
        hooli_request(
            1,
            2,
            "Tue Nov 3, 2pm ET",
            "2030-11-03T19:00:00Z",
            "2030-11-03T19:45:00Z"
        )
    );
    assert_eq!(
        stored[1].request,
        hooli_request(
            2,
            2,
            "Wed Nov 4, 10am ET",
            "2030-11-04T15:00:00Z",
            "2030-11-04T15:30:00Z"
        )
    );
    for proposal in &stored {
        assert_eq!(proposal.status, "proposed");
        parked(&pool, proposal).await;
    }
    assert!(
        received(&server, EVENTS).await.is_empty(),
        "Calendar is never called"
    );

    // Running the match again adds nothing.
    let again = run_match(&pool, classification).await;
    assert_eq!(again["status"], "applied");
    assert_eq!(
        count(&pool, "SELECT count(*) FROM interview_events").await,
        2
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_runs WHERE workflow = 'propose_interview'"
        )
        .await,
        2
    );

    // A rejection proposes nothing, even with a future time.
    let future = json!([time(
        "Dec 1, 5pm UTC",
        Some("UTC"),
        Some("2030-12-01T17:00:00Z"),
        None
    )]);
    let initech = seed_application(&pool, "Initech", "Data Engineer").await;
    let (_, rejection) = seed_email(
        &pool,
        "initech-1",
        &Email {
            company: "Initech",
            role: "Data Engineer",
            class: "rejection",
            confidence: 0.9,
            times: future.clone(),
        },
    )
    .await;
    assert_eq!(run_match(&pool, rejection).await["status"], "applied");
    assert!(proposals(&pool, initech).await.is_empty());

    // An unsure match waits in review and proposes nothing until it is accepted.
    let globex = seed_application(&pool, "Globex", "Platform Engineer").await;
    let (_, unsure) = seed_email(
        &pool,
        "globex-1",
        &Email {
            company: "Globex",
            role: "Platform Engineer",
            class: "interview_invite",
            confidence: 0.6,
            times: future,
        },
    )
    .await;
    let review = run_match(&pool, unsure).await;
    assert_eq!(review["status"], "review", "{review}");
    assert_eq!(review["review_kind"], "email.match");
    assert!(proposals(&pool, globex).await.is_empty());
    let item = review["review_item_id"].as_i64().expect("a review item");
    let (status, body) = post(
        &client,
        &format!("/review-items/{item}/accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let accepted = proposals(&pool, globex).await;
    assert_eq!(accepted.len(), 1);
    assert_eq!(
        accepted[0].request["summary"],
        "Interview: Globex — Technical interview"
    );
    parked(&pool, &accepted[0]).await;
    assert!(
        received(&server, EVENTS).await.is_empty(),
        "Calendar is never called"
    );
}

#[sqlx::test]
async fn approve_creates_the_event_once_and_links_it(pool: PgPool) {
    let server = MockServer::start().await;
    calendar_ok(&server).await;
    let (client, _fake) = app(&pool, &server).await;
    let hooli = seed_application(&pool, "Hooli", "Backend Engineer").await;
    let (email, classification) = seed_email(&pool, "hooli-1", &hooli_email(two_slots())).await;
    run_match(&pool, classification).await;
    let stored = proposals(&pool, hooli).await;
    assert_eq!(stored.len(), 2);
    let first = parked(&pool, &stored[0]).await;
    let second = parked(&pool, &stored[1]).await;

    // Approve the first slot with a later end.
    let mut edited = stored[0].request.clone();
    edited["end"] = json!("2030-11-03T20:00:00Z");
    let (status, body) = post(
        &client,
        &format!("/pending-actions/{first}/approve"),
        Some(json!({"input": edited})),
    )
    .await;
    assert_eq!(status, Status::Ok, "{body}");
    let output = until(&pool, stored[0].run(), "succeeded").await;
    assert_eq!(
        output,
        json!({"interview_event_id": stored[0].id, "application_id": hooli,
               "status": "created", "google_event_id": "evt-1", "html_link": LINK})
    );

    let requests = received(&server, EVENTS).await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url.query(), Some("sendUpdates=none"));
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
        Some(format!("Bearer {}", fake("access")).as_str())
    );
    let sent: Value = serde_json::from_slice(&request.body).expect("a JSON body");
    assert_eq!(
        sent,
        json!({
            "summary": edited["summary"],
            "description": edited["description"],
            "start": {"dateTime": "2030-11-03T19:00:00Z", "timeZone": "America/New_York"},
            "end": {"dateTime": "2030-11-03T20:00:00Z", "timeZone": "America/New_York"},
            "reminders": {"useDefault": true},
        })
    );

    let created = &proposals(&pool, hooli).await[0];
    assert_eq!(created.status, "created");
    assert_eq!(created.google_event_id.as_deref(), Some("evt-1"));
    assert_eq!(created.html_link.as_deref(), Some(LINK));
    assert_eq!(created.start_at, Some(instant("2030-11-03T19:00:00Z")));
    assert_eq!(created.end_at, Some(instant("2030-11-03T20:00:00Z")));
    assert_eq!(created.error, None);
    assert_eq!(action_status(&pool, first).await, "executed");

    let events = get(&client, &format!("/applications/{hooli}/events")).await;
    let scheduled: Vec<&Value> = events
        .as_array()
        .expect("an array")
        .iter()
        .filter(|event| event["kind"] == "interview_scheduled")
        .collect();
    assert_eq!(scheduled.len(), 1, "{events}");
    assert_eq!(
        scheduled[0]["body"],
        "Interview: Hooli — Technical interview · 2030-11-03 14:00–15:00 America/New_York"
    );
    assert_eq!(scheduled[0]["source_email"]["id"], email);
    assert_eq!(scheduled[0]["source_email"]["subject"], SUBJECT);

    // A second approve is refused, and Google is called once.
    let (status, _) = post(
        &client,
        &format!("/pending-actions/{first}/approve"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, Status::Conflict);
    assert_eq!(received(&server, EVENTS).await.len(), 1);

    // The sibling slot is untouched until it is approved too.
    assert_eq!(action_status(&pool, second).await, "pending");
    assert_eq!(proposals(&pool, hooli).await[1].status, "proposed");
}

#[sqlx::test]
async fn reject_never_calls_calendar(pool: PgPool) {
    let server = MockServer::start().await;
    calendar_ok(&server).await;
    let (client, _fake) = app(&pool, &server).await;
    let hooli = seed_application(&pool, "Hooli", "Backend Engineer").await;
    let (_, classification) = seed_email(&pool, "hooli-1", &hooli_email(two_slots())).await;
    run_match(&pool, classification).await;
    let stored = proposals(&pool, hooli).await;
    let first = parked(&pool, &stored[0]).await;
    let second = parked(&pool, &stored[1]).await;

    let (status, body) = post(&client, &format!("/pending-actions/{first}/reject"), None).await;
    assert_eq!(status, Status::Ok, "{body}");
    until(&pool, stored[0].run(), "rejected").await;
    let after = proposals(&pool, hooli).await;
    assert_eq!(after[0].status, "rejected");
    assert_eq!(after[1].status, "proposed");
    assert_eq!(action_status(&pool, first).await, "rejected");
    assert_eq!(action_status(&pool, second).await, "pending");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(received(&server, EVENTS).await.is_empty());
}

#[sqlx::test]
async fn calendar_failures_end_the_proposal(pool: PgPool) {
    let server = MockServer::start().await;
    let (client, _fake) = app(&pool, &server).await;

    // One parked slot per scenario.
    let mut slots = Vec::new();
    for (day, company) in ["Hooli", "Initech", "Globex", "Umbrella"]
        .iter()
        .enumerate()
    {
        let application = seed_application(&pool, company, "Backend Engineer").await;
        let start = format!("2030-11-{:02}T15:00:00Z", 10 + day);
        let times = json!([time("a time", Some("UTC"), Some(&start), None)]);
        let (_, classification) = seed_email(
            &pool,
            &format!("{company}-1"),
            &Email {
                company,
                role: "Backend Engineer",
                class: "interview_invite",
                confidence: 0.9,
                times,
            },
        )
        .await;
        assert_eq!(run_match(&pool, classification).await["status"], "applied");
        let stored = proposals(&pool, application).await;
        assert_eq!(stored.len(), 1, "{company}");
        let action = parked(&pool, &stored[0]).await;
        slots.push((application, stored.into_iter().next().unwrap(), action));
    }
    let approve = |action: i64| {
        let client = &client;
        async move {
            let (status, body) = post(
                client,
                &format!("/pending-actions/{action}/approve"),
                Some(json!({})),
            )
            .await;
            assert_eq!(status, Status::Ok, "{body}");
        }
    };

    // Calendar answers 500: the proposal, its run and its action fail after one request.
    let (application, proposal, action) = &slots[0];
    Mock::given(method("POST"))
        .and(path(EVENTS))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    approve(*action).await;
    until(&pool, proposal.run(), "failed").await;
    let failed = &proposals(&pool, *application).await[0];
    assert_eq!(failed.status, "failed");
    let error = failed.error.as_deref().expect("an error");
    assert!(error.contains("HTTP 500"), "{error}");
    assert!(!error.contains(&fake("access")), "{error}");
    assert_eq!(action_status(&pool, *action).await, "failed");
    assert_eq!(received(&server, EVENTS).await.len(), 1);

    // A refused token is refreshed at the token endpoint and the insert retried once.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path(EVENTS))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    calendar_ok(&server).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": fake("fresh"), "expires_in": 3600, "token_type": "Bearer"}),
        ))
        .mount(&server)
        .await;
    let (application, proposal, action) = &slots[1];
    approve(*action).await;
    until(&pool, proposal.run(), "succeeded").await;
    assert_eq!(proposals(&pool, *application).await[0].status, "created");
    let tokens: Vec<Option<String>> = received(&server, EVENTS)
        .await
        .iter()
        .map(|request| {
            request
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(
        tokens,
        [
            Some(format!("Bearer {}", fake("access"))),
            Some(format!("Bearer {}", fake("fresh")))
        ]
    );
    assert_eq!(received(&server, "/token").await.len(), 1);

    // No Google account: the proposal fails without a request.
    server.reset().await;
    calendar_ok(&server).await;
    sqlx::query("DELETE FROM google_accounts")
        .execute(&pool)
        .await
        .expect("disconnect");
    let (application, proposal, action) = &slots[2];
    approve(*action).await;
    until(&pool, proposal.run(), "failed").await;
    let failed = &proposals(&pool, *application).await[0];
    assert_eq!(failed.status, "failed");
    let error = failed.error.as_deref().expect("an error");
    assert!(error.contains("no Google account is connected"), "{error}");
    assert!(received(&server, EVENTS).await.is_empty());

    // The application was deleted before approval: the run skips without a request.
    let (application, proposal, action) = &slots[3];
    sqlx::query("DELETE FROM applications WHERE id = $1")
        .bind(application)
        .execute(&pool)
        .await
        .expect("delete the application");
    approve(*action).await;
    let output = until(&pool, proposal.run(), "succeeded").await;
    assert_eq!(output["status"], "skipped", "{output}");
    assert_eq!(output["interview_event_id"], proposal.id);
    assert_eq!(action_status(&pool, *action).await, "approved");
    assert!(received(&server, EVENTS).await.is_empty());
}
