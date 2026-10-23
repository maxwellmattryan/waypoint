//! The kill switch (`GET`/`PUT /kill-switch`) and delete-all (`DELETE /data`): pausing and
//! unpausing runs, the worker honouring the switch, the Google revocation against a mocked
//! Google (wiremock) and the guards. Model calls go to `fake-claude.sh`.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, PgPool};
use tokio::sync::{Notify, watch};
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::crypto::{self, TokenKey};
use waypoint_api::events::{Event, EventBus, Subscriber};
use waypoint_api::google::{self, Google, GoogleRevocation};
use waypoint_api::jobs::{self, Wake, Worker};
use waypoint_api::kill_switch::{self, USER_TABLES};
use waypoint_api::runs::{self, RunStatus, Runtime, StepContext, StepError, Workflow};
use waypoint_api::tools::Registry;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TIMEOUT: Duration = Duration::from_secs(10);
const KEY: [u8; 32] = [9; 32];

/// A fake refresh token, built at runtime so it never looks like a committed secret.
fn fake_refresh() -> String {
    format!("fake-refresh-{}", "for-tests")
}

fn google_config(server: &MockServer) -> google::Config {
    google::Config {
        client_id: Some("test-client.apps.googleusercontent.com".into()),
        client_secret: Some(format!("fake-client-secret-{}", "for-tests")),
        token_key: Some(TokenKey::new(KEY)),
        auth_url: format!("{}/auth", server.uri()),
        token_url: format!("{}/token", server.uri()),
        revoke_url: format!("{}/revoke", server.uri()),
        redirect_uri: google::config::redirect_uri(8000),
        // Startup catch-up may sync against the mock, never the real Gmail.
        gmail: waypoint_api::gmail::Config {
            base_url: server.uri(),
            ..Default::default()
        },
    }
}

/// Stores a connected Google account whose refresh token is [`fake_refresh`].
async fn seed_google(pool: &PgPool) {
    let key = TokenKey::new(KEY);
    sqlx::query(
        "INSERT INTO google_accounts \
             (refresh_token, access_token, access_token_expires_at, scopes, status) \
         VALUES ($1, $2, now() + interval '1 hour', $3, 'connected')",
    )
    .bind(crypto::seal(
        &key,
        google::REFRESH_TOKEN_AAD,
        fake_refresh().as_bytes(),
    ))
    .bind(crypto::seal(&key, google::ACCESS_TOKEN_AAD, b"access"))
    .bind(google::SCOPES.map(String::from).to_vec())
    .execute(pool)
    .await
    .expect("seed the account");
}

/// One row in most user tables: an application, a run with a step, a job, a model call
/// an OAuth state, a memory and a learning signal.
async fn seed_data(pool: &PgPool) {
    for sql in [
        "INSERT INTO applications (company, title) VALUES ('Acme', 'Engineer')",
        "INSERT INTO memories (kind, topic, content) VALUES ('fact', 'skill', 'Rust')",
        "INSERT INTO memory_signals (kind, run_id, pending_action_id, subject, drafted) \
         VALUES ('rejection', 1, 1, 'send', '{}')",
        "INSERT INTO jobs (kind, payload, status) VALUES ('test_prompt', '{}', 'succeeded')",
        "INSERT INTO agent_runs (workflow, input, status) VALUES ('w', '{}', 'failed')",
        "INSERT INTO run_steps (run_id, ordinal, name) SELECT id, 0, 'a' FROM agent_runs",
        "INSERT INTO claude_calls (purpose, status, duration_ms) VALUES ('test', 'ok', 1)",
        "INSERT INTO google_oauth_states (state, code_verifier) VALUES ('s', 'v')",
    ] {
        sqlx::query(sql).execute(pool).await.expect(sql);
    }
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

async fn paused(pool: &PgPool) -> bool {
    kill_switch::is_paused(pool).await.expect("read the switch")
}

async fn insert_run(pool: &PgPool, status: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO agent_runs (workflow, input, status) VALUES ('w', '{}', $1) RETURNING id",
    )
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert a run")
}

async fn run_status(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM agent_runs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read the run")
}

/// The form fields of a recorded request.
fn form(request: &Request) -> HashMap<String, String> {
    reqwest::Url::parse(&format!(
        "http://x/?{}",
        String::from_utf8_lossy(&request.body)
    ))
    .expect("form body")
    .query_pairs()
    .into_owned()
    .collect()
}

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

async fn client(pool: &PgPool, google: google::Config) -> Client {
    // A sync that just finished, so the worker's startup catch-up never queues a Gmail
    // sync for the account a test seeds; `DELETE /data` must empty this table too.
    sqlx::query(
        "INSERT INTO gmail_sync_state (id, last_finished_at) VALUES (true, now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed the Gmail sync state");
    let rocket = waypoint_api::rocket(
        rocket::Config::figment(),
        pool.clone(),
        ClaudeRunner::new("claude", Duration::from_secs(1)),
        common::fast_config(),
        google,
        waypoint_api::capture::Config::default(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
}

fn bus(client: &Client) -> &EventBus {
    client
        .rocket()
        .state::<EventBus>()
        .expect("the bus is managed")
}

async fn put_switch<'c>(client: &'c Client, paused: bool) -> LocalResponse<'c> {
    client
        .put("/kill-switch")
        .header(host())
        .header(ContentType::JSON)
        .body(json!({"paused": paused}).to_string())
        .dispatch()
        .await
}

async fn delete_data<'c>(client: &'c Client, body: &str) -> LocalResponse<'c> {
    client
        .delete("/data")
        .header(host())
        .header(ContentType::JSON)
        .body(body)
        .dispatch()
        .await
}

/// Every event published within `wait`.
async fn drain(subscriber: &mut Subscriber, wait: Duration) -> Vec<Event> {
    common::events(subscriber, wait, |_| false).await
}

/// Starts a worker on `bus` and `wake` against the fake in `success` mode.
fn spawn_worker(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    runtime: Runtime,
    config: jobs::Config,
) -> (common::Fake, watch::Sender<bool>) {
    let fake = common::fake("success", TIMEOUT);
    let (stop, stopped) = watch::channel(false);
    Worker {
        pool: pool.clone(),
        runner: fake.runner.clone(),
        bus: bus.clone(),
        config,
        wake: wake.0.clone(),
        runtime,
        google: Google::new(google::Config::default()),
    }
    .spawn(stopped);
    (fake, stop)
}

async fn until_finished(subscriber: &mut Subscriber, id: i64) -> Vec<Event> {
    common::events(subscriber, TIMEOUT, |event| {
        matches!(event, Event::JobSucceeded { job } | Event::JobFailed { job } if job.id == id)
    })
    .await
}

/// A step row as `(status, attempts)`, by ordinal.
async fn steps(pool: &PgPool, run_id: i64) -> Vec<(String, i32)> {
    sqlx::query_as("SELECT status, attempts FROM run_steps WHERE run_id = $1 ORDER BY ordinal")
        .bind(run_id)
        .fetch_all(pool)
        .await
        .expect("read steps")
}

/// Two steps that pass their input on. `first` counts its calls and, when built with a
/// switch, turns the kill switch on before it returns.
struct Pausing {
    first_calls: Arc<AtomicUsize>,
    switch: Option<(PgPool, EventBus, Wake)>,
}

#[rocket::async_trait]
impl Workflow for Pausing {
    fn name(&self) -> &'static str {
        "pausing"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["first", "second"]
    }

    async fn run_step(
        &self,
        _ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        if step == "first" {
            self.first_calls.fetch_add(1, Ordering::SeqCst);
            if let Some((pool, bus, wake)) = &self.switch {
                kill_switch::set_paused(pool, bus, wake, true)
                    .await
                    .map_err(|err| StepError::Fail(err.to_string()))?;
            }
        }
        Ok(json!({step: input}))
    }
}

fn runtime_of(workflow: impl Workflow + 'static) -> Runtime {
    Runtime::new(vec![Arc::new(workflow)], Registry::default()).expect("valid runtime")
}

#[sqlx::test]
async fn fresh_database_is_unpaused(pool: PgPool) {
    let state = kill_switch::get(&pool).await.expect("read the switch");
    assert!(!state.paused);
    assert_eq!(count(&pool, "kill_switch").await, 1);
    let id = insert_run(&pool, "paused").await;
    assert_eq!(run_status(&pool, id).await, "paused");
}

#[sqlx::test]
async fn put_toggles_and_is_idempotent(pool: PgPool) {
    let client = client(&pool, google::Config::default()).await;
    let mut subscriber = bus(&client).subscribe();

    let response = client.get("/kill-switch").header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Ok);
    let initial: Value = response.into_json().await.expect("json");
    assert_eq!(initial["paused"], false);
    assert!(initial["updated_at"].is_string(), "{initial}");

    let response = put_switch(&client, true).await;
    assert_eq!(response.status(), Status::Ok);
    let on: Value = response.into_json().await.expect("json");
    assert_eq!(on["paused"], true);
    assert_ne!(on["updated_at"], initial["updated_at"]);
    let events = drain(&mut subscriber, Duration::from_millis(200)).await;
    let switched: Vec<bool> = events
        .iter()
        .filter_map(|event| match event {
            Event::KillSwitchUpdated { kill_switch } => Some(kill_switch.paused),
            _ => None,
        })
        .collect();
    assert_eq!(switched, [true]);

    // The same value again: nothing changes and nothing is published.
    let response = put_switch(&client, true).await;
    assert_eq!(response.status(), Status::Ok);
    let again: Value = response.into_json().await.expect("json");
    assert_eq!(again, on);
    let events = drain(&mut subscriber, Duration::from_millis(200)).await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::KillSwitchUpdated { .. })),
        "{events:?}"
    );

    let response = client.get("/kill-switch").header(host()).dispatch().await;
    assert_eq!(response.into_json::<Value>().await, Some(on));

    let response = put_switch(&client, false).await;
    assert_eq!(
        response.into_json::<Value>().await.expect("json")["paused"],
        false
    );
    assert!(!paused(&pool).await);
}

#[sqlx::test]
async fn pause_and_unpause_move_runs(pool: PgPool) {
    let queued = [
        insert_run(&pool, "queued").await,
        insert_run(&pool, "queued").await,
    ];
    let running = insert_run(&pool, "running").await;
    let failed = insert_run(&pool, "failed").await;
    let bus = EventBus::new();
    let wake = Wake::default();
    let mut subscriber = bus.subscribe();

    for (on, from, to) in [(true, "queued", "paused"), (false, "paused", "queued")] {
        let state = kill_switch::set_paused(&pool, &bus, &wake, on)
            .await
            .expect("set the switch");
        assert_eq!(state.paused, on);
        for id in queued {
            assert_eq!(
                run_status(&pool, id).await,
                to,
                "run {id} went {from} → {to}"
            );
        }
        assert_eq!(run_status(&pool, running).await, "running");
        assert_eq!(run_status(&pool, failed).await, "failed");

        // Published after the commit: every run update matches the stored row.
        let events = drain(&mut subscriber, Duration::from_millis(200)).await;
        let names: Vec<&str> = events.iter().map(Event::name).collect();
        assert_eq!(
            names,
            ["run.updated", "run.updated", "kill_switch.updated"],
            "{events:?}"
        );
        let mut moved = BTreeSet::new();
        for event in &events {
            match event {
                Event::RunUpdated { run } => {
                    assert_eq!(serde_json::to_value(run.status).unwrap(), json!(to));
                    moved.insert(run.id);
                }
                Event::KillSwitchUpdated { kill_switch } => {
                    assert_eq!(kill_switch, &state);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(moved, BTreeSet::from(queued));
    }
}

#[sqlx::test]
async fn paused_runs_do_not_start_until_unpaused(pool: PgPool) {
    let first_calls = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_of(Pausing {
        first_calls: first_calls.clone(),
        switch: None,
    });
    let bus = EventBus::new();
    let wake = Wake::default();
    let mut subscriber = bus.subscribe();
    let _worker = spawn_worker(&pool, &bus, &wake, runtime.clone(), common::fast_config());

    kill_switch::set_paused(&pool, &bus, &wake, true)
        .await
        .expect("pause");
    let started = runs::start(&pool, &bus, &wake, &runtime, "pausing", json!({}))
        .await
        .expect("start the run");
    assert_eq!(started.status, RunStatus::Paused);
    let run_job = started.job_id.expect("job_id");

    // Other job kinds still run.
    let prompt = jobs::enqueue(
        &pool,
        &bus,
        &wake,
        jobs::TEST_PROMPT,
        json!({"prompt": "hi"}),
    )
    .await
    .expect("enqueue");
    let events = until_finished(&mut subscriber, prompt.id).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::JobSucceeded { job } if job.id == prompt.id)),
        "{events:?}"
    );
    // Give the worker a few more polls to (wrongly) pick the run's job up.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let job = jobs::get(&pool, run_job).await.expect("get").expect("job");
    assert_eq!((job.status.as_str(), job.attempts), ("queued", 0));
    assert_eq!(run_status(&pool, started.id).await, "paused");
    assert_eq!(first_calls.load(Ordering::SeqCst), 0);

    kill_switch::set_paused(&pool, &bus, &wake, false)
        .await
        .expect("unpause");
    until_finished(&mut subscriber, run_job).await;
    assert_eq!(run_status(&pool, started.id).await, "succeeded");
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
}

#[sqlx::test]
async fn running_run_stops_at_next_step(pool: PgPool) {
    let bus = EventBus::new();
    let wake = Wake::default();
    let first_calls = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_of(Pausing {
        first_calls: first_calls.clone(),
        switch: Some((pool.clone(), bus.clone(), wake.clone())),
    });
    let mut subscriber = bus.subscribe();
    let _worker = spawn_worker(&pool, &bus, &wake, runtime.clone(), common::fast_config());

    let started = runs::start(&pool, &bus, &wake, &runtime, "pausing", json!({}))
        .await
        .expect("start the run");
    let run_job = started.job_id.expect("job_id");
    let events = common::events(
        &mut subscriber,
        TIMEOUT,
        |event| matches!(event, Event::JobDeferred { job } if job.id == run_job),
    )
    .await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::QueuePaused { .. })),
        "the queue keeps running: {events:?}"
    );

    let stopped = runs::get_detail(&pool, started.id)
        .await
        .expect("read")
        .expect("the run");
    assert_eq!(stopped.run.status, RunStatus::Paused);
    assert_eq!(stopped.run.error, None);
    assert_eq!(
        steps(&pool, started.id).await,
        [("succeeded".to_string(), 1), ("pending".to_string(), 0)]
    );
    let job = jobs::get(&pool, run_job).await.expect("get").expect("job");
    assert_eq!(
        (job.status.as_str(), job.attempts),
        ("queued", 0),
        "the attempt is given back"
    );

    kill_switch::set_paused(&pool, &bus, &wake, false)
        .await
        .expect("unpause");
    until_finished(&mut subscriber, run_job).await;
    assert_eq!(run_status(&pool, started.id).await, "succeeded");
    assert_eq!(
        steps(&pool, started.id).await,
        [("succeeded".to_string(), 1), ("succeeded".to_string(), 1)],
        "step 1 isn't run again"
    );
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
}

#[sqlx::test]
async fn resume_while_paused_stays_paused(pool: PgPool) {
    let failed = insert_run(&pool, "failed").await;
    let bus = EventBus::new();
    let wake = Wake::default();
    kill_switch::set_paused(&pool, &bus, &wake, true)
        .await
        .expect("pause");

    let resumed = runs::resume(&pool, &bus, &wake, failed)
        .await
        .expect("resume")
        .expect("a failed run resumes");
    assert_eq!(resumed.status, RunStatus::Paused);
    assert_eq!(resumed.finished_at, None);
    assert!(resumed.job_id.is_some());
    assert_eq!(
        jobs::claim(&pool).await.expect("claim"),
        None,
        "its agent_run job waits"
    );
}

#[sqlx::test]
async fn delete_requires_exact_confirmation(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client(&pool, google_config(&server)).await;
    seed_google(&pool).await;
    seed_data(&pool).await;

    for phrase in [
        "",
        "Delete all data",
        "delete all data ",
        " delete all data",
        "delete",
    ] {
        let response = delete_data(&client, &json!({"confirmation": phrase}).to_string()).await;
        assert_eq!(response.status(), Status::UnprocessableEntity, "{phrase:?}");
        assert_eq!(
            response.into_json::<Value>().await,
            Some(json!({"errors": [{
                "field": "confirmation",
                "message": "type \"delete all data\" to confirm",
            }]})),
            "{phrase:?}"
        );
    }
    for body in [
        "",
        "{",
        "{}",
        r#"{"confirmation": 1}"#,
        r#"{"confirmation": "delete all data", "also": true}"#,
    ] {
        let response = delete_data(&client, body).await;
        assert_eq!(response.status(), Status::BadRequest, "{body:?}");
        let errors: Value = response.into_json().await.expect("json");
        assert_eq!(errors["errors"][0]["field"], "body", "{body:?}");
    }

    assert!(!paused(&pool).await);
    assert_eq!(count(&pool, "google_accounts").await, 1);
    assert_eq!(count(&pool, "applications").await, 1);
    assert_eq!(count(&pool, "jobs").await, 1);
    server.verify().await;
}

/// Answers 200 to the revoke, after recording how many `google_accounts` rows exist at
/// that moment (read over a fresh connection, since the responder is synchronous).
struct CheckRowThenOk {
    database: sqlx::postgres::PgConnectOptions,
    seen: Arc<Mutex<Option<i64>>>,
}

impl Respond for CheckRowThenOk {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let options = self.database.clone();
        let rows = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(async move {
                    let mut conn = PgConnection::connect_with(&options).await.expect("connect");
                    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM google_accounts")
                        .fetch_one(&mut conn)
                        .await
                        .expect("count");
                    rows
                })
        })
        .join()
        .expect("the check ran");
        *self.seen.lock().unwrap() = Some(rows);
        ResponseTemplate::new(200)
    }
}

#[sqlx::test]
async fn delete_revokes_then_truncates(pool: PgPool) {
    let server = MockServer::start().await;
    let seen = Arc::new(Mutex::new(None));
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .respond_with(CheckRowThenOk {
            database: (*pool.connect_options()).clone(),
            seen: seen.clone(),
        })
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&pool, google_config(&server)).await;
    seed_google(&pool).await;
    seed_data(&pool).await;
    let queued = insert_run(&pool, "queued").await;
    let mut subscriber = bus(&client).subscribe();

    let response = delete_data(&client, r#"{"confirmation": "delete all data"}"#).await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.into_json::<Value>().await,
        Some(json!({"google": "revoked"}))
    );

    assert_eq!(
        *seen.lock().unwrap(),
        Some(1),
        "revoked before the truncate"
    );
    let revokes = server.received_requests().await.expect("recording is on");
    assert_eq!(form(&revokes[0])["token"], fake_refresh());
    for table in USER_TABLES {
        assert_eq!(count(&pool, table).await, 0, "{table}");
    }
    assert_eq!(count(&pool, "kill_switch").await, 1);
    assert!(paused(&pool).await, "agents stay paused");

    let events = drain(&mut subscriber, Duration::from_millis(200)).await;
    let names: Vec<&str> = events.iter().map(Event::name).collect();
    assert_eq!(
        names,
        ["run.updated", "kill_switch.updated", "data.deleted"],
        "the queued run was paused first: {events:?}"
    );
    assert!(matches!(&events[0], Event::RunUpdated { run } if run.id == queued));

    // RESTART IDENTITY: the next row starts from 1, as on a fresh install.
    let job = jobs::enqueue(&pool, &EventBus::new(), &Wake::default(), "noop", json!({}))
        .await
        .expect("enqueue");
    assert_eq!(job.id, 1);
    server.verify().await;
}

#[sqlx::test]
async fn delete_reports_google_outcome(pool: PgPool) {
    let server = MockServer::start().await;
    let client = client(&pool, google_config(&server)).await;
    let cases = [
        (Some(200), "revoked"),
        (Some(500), "not_revoked"),
        (None, "not_connected"),
    ];
    for (google_answers, expected) in cases {
        server.reset().await;
        if let Some(status) = google_answers {
            Mock::given(method("POST"))
                .and(path("/revoke"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            seed_google(&pool).await;
        }
        seed_data(&pool).await;

        let response = delete_data(&client, r#"{"confirmation": "delete all data"}"#).await;
        assert_eq!(response.status(), Status::Ok, "{expected}");
        assert_eq!(
            response.into_json::<Value>().await,
            Some(json!({"google": expected}))
        );
        assert_eq!(count(&pool, "google_accounts").await, 0, "{expected}");
        assert_eq!(count(&pool, "applications").await, 0, "{expected}");
        server.verify().await;
    }

    // Not configured (no token key): the stored token can't be read, so nothing is revoked.
    seed_google(&pool).await;
    let unconfigured = Google::new(google::Config::default());
    let outcome = google::revoke_stored(&pool, &unconfigured)
        .await
        .expect("revoke");
    assert_eq!(outcome, GoogleRevocation::NotRevoked);
}

#[sqlx::test]
async fn user_tables_cover_the_schema(pool: PgPool) {
    let tables: BTreeSet<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
         AND table_name NOT IN ('_sqlx_migrations', 'kill_switch')",
    )
    .fetch_all(&pool)
    .await
    .expect("list tables")
    .into_iter()
    .collect();
    let listed: BTreeSet<String> = USER_TABLES.iter().map(|t| t.to_string()).collect();
    assert_eq!(listed, tables, "USER_TABLES must name every user table");
}

#[sqlx::test]
async fn guards_refuse_cross_site_and_forms(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = client(&pool, google_config(&server)).await;
    seed_google(&pool).await;
    seed_data(&pool).await;
    let cross_site = || Header::new("Sec-Fetch-Site", "cross-site");
    let confirm = json!({"confirmation": "delete all data"}).to_string();

    let refused = [
        client
            .put("/kill-switch")
            .header(host())
            .header(cross_site())
            .header(ContentType::JSON)
            .body(json!({"paused": true}).to_string()),
        client
            .put("/kill-switch")
            .header(host())
            .header(ContentType::Form)
            .body("paused=true"),
        // What any web page can auto-submit: a form POST that Rocket turns into PUT/DELETE.
        client
            .post("/kill-switch")
            .header(host())
            .header(ContentType::Form)
            .body("_method=put&paused=true"),
        client
            .delete("/data")
            .header(host())
            .header(cross_site())
            .header(ContentType::JSON)
            .body(confirm.clone()),
        client
            .post("/data")
            .header(host())
            .header(ContentType::Form)
            .body("_method=delete&confirmation=delete+all+data"),
        // A non-local Host on all three routes.
        client
            .get("/kill-switch")
            .header(Header::new("Host", "evil.example")),
        client
            .put("/kill-switch")
            .header(Header::new("Host", "evil.example"))
            .header(ContentType::JSON)
            .body(json!({"paused": true}).to_string()),
        client
            .delete("/data")
            .header(Header::new("Host", "evil.example"))
            .header(ContentType::JSON)
            .body(confirm.clone()),
    ];
    for (i, request) in refused.into_iter().enumerate() {
        assert_eq!(
            request.dispatch().await.status(),
            Status::Forbidden,
            "request {i}"
        );
    }

    assert!(!paused(&pool).await);
    assert_eq!(count(&pool, "google_accounts").await, 1);
    assert_eq!(count(&pool, "applications").await, 1);
    server.verify().await;
}

/// One step that tells the test it started, then waits for the test to let it finish.
struct Blocking {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[rocket::async_trait]
impl Workflow for Blocking {
    fn name(&self) -> &'static str {
        "blocking"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["wait", "after"]
    }

    async fn run_step(
        &self,
        _ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        if step == "wait" {
            self.started.notify_one();
            self.release.notified().await;
        }
        Ok(input)
    }
}

#[sqlx::test]
async fn worker_survives_truncate_mid_run(pool: PgPool) {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let runtime = runtime_of(Blocking {
        started: started.clone(),
        release: release.clone(),
    });
    let bus = EventBus::new();
    let wake = Wake::default();
    let mut subscriber = bus.subscribe();
    // One job at a time, so a job stuck recording its outcome would block every other.
    let config = jobs::Config {
        concurrency: 1,
        ..common::fast_config()
    };
    let _worker = spawn_worker(&pool, &bus, &wake, runtime.clone(), config);

    runs::start(&pool, &bus, &wake, &runtime, "blocking", json!({}))
        .await
        .expect("start the run");
    tokio::time::timeout(TIMEOUT, started.notified())
        .await
        .expect("the step started");

    let server = MockServer::start().await;
    let google = Google::new(google_config(&server));
    let outcome = kill_switch::delete_all_data(&pool, &bus, &wake, &google)
        .await
        .expect("delete all data");
    assert_eq!(outcome, GoogleRevocation::NotConnected);
    assert_eq!(count(&pool, "jobs").await, 0);
    // Keep new jobs off the old job's id, so a worker retrying its write can't land on
    // one of them and get unstuck by accident.
    sqlx::query("SELECT setval('jobs_id_seq', 100)")
        .execute(&pool)
        .await
        .expect("move the sequence");

    // The step finishes after its rows are gone; give the worker time to try recording it.
    release.notify_one();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let prompt = jobs::enqueue(
        &pool,
        &bus,
        &wake,
        jobs::TEST_PROMPT,
        json!({"prompt": "hi"}),
    )
    .await
    .expect("enqueue");
    let events = until_finished(&mut subscriber, prompt.id).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::JobSucceeded { job } if job.id == prompt.id)),
        "the worker still claims jobs: {events:?}"
    );
}
