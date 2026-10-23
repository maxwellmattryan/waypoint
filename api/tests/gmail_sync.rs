//! Gmail sync against a fake Gmail (wiremock): backfill, incremental, resync, the size
//! cap, single-flight, startup catch-up, Gmail's errors and the `/gmail/sync` routes. The
//! raw messages are the synthetic `api/fixtures/emails/*.eml` files, with their file stems
//! as Gmail ids.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE;
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::crypto::{self, TokenKey};
use waypoint_api::events::{Event, EventBus, Subscriber};
use waypoint_api::gmail::{self, sync};
use waypoint_api::google::{self, Google};
use waypoint_api::jobs::{self, Job, Wake, Worker};
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TIMEOUT: Duration = Duration::from_secs(15);
const KEY: [u8; 32] = [5; 32];
const ME: &str = "me@example.com";
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/emails");

/// A fake token, built at runtime so it never looks like a committed secret.
fn fake(kind: &str) -> String {
    format!("fake-{kind}-{}", "for-tests")
}

fn eml(stem: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{stem}.eml")).expect("fixture exists")
}

/// The `messages.get?format=raw` body for a fixture.
fn message(stem: &str) -> Value {
    message_with(stem, &["INBOX"])
}

fn message_with(stem: &str, labels: &[&str]) -> Value {
    json!({
        "id": stem,
        "threadId": format!("thread-{stem}"),
        "labelIds": labels,
        "raw": URL_SAFE.encode(eml(stem)),
        "sizeEstimate": 1,
    })
}

/// What the fake Gmail serves. Tests change it between runs.
#[derive(Default)]
struct Mailbox {
    email: String,
    /// `users.getProfile`'s `historyId`.
    profile_history_id: String,
    /// `messages.list`, newest first.
    listed: Vec<String>,
    /// `history.list`'s `messagesAdded`, as `(id, labels)`.
    history: Vec<(String, Vec<String>)>,
    /// `history.list`'s `historyId`.
    history_page_id: String,
    /// `history.list` answers 404.
    history_expired: bool,
    /// `messages.get` bodies by id; a missing id answers 404.
    messages: HashMap<String, Value>,
}

#[derive(Clone)]
struct FakeGmail(Arc<Mutex<Mailbox>>);

impl FakeGmail {
    fn edit(&self, change: impl FnOnce(&mut Mailbox)) {
        change(&mut self.0.lock().unwrap());
    }

    /// Adds fixtures to `messages.list` (in this order, newest first) and `messages.get`.
    fn list(&self, stems: &[&str]) {
        self.edit(|mailbox| {
            for stem in stems {
                mailbox.listed.push((*stem).into());
                mailbox.messages.insert((*stem).into(), message(stem));
            }
        });
    }
}

fn query(request: &Request) -> HashMap<String, String> {
    request.url.query_pairs().into_owned().collect()
}

/// Pages `items` by the request's `pageToken` (an offset) and `maxResults`.
fn page<T: Clone>(items: &[T], query: &HashMap<String, String>) -> (Vec<T>, Option<String>) {
    let start: usize = query
        .get("pageToken")
        .map_or(0, |token| token.parse().unwrap());
    let max: usize = query["maxResults"].parse().unwrap();
    let end = (start + max).min(items.len());
    let next = (end < items.len()).then(|| end.to_string());
    (items[start.min(end)..end].to_vec(), next)
}

impl Respond for FakeGmail {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mailbox = self.0.lock().unwrap();
        let query = query(request);
        let rest = request
            .url
            .path()
            .strip_prefix("/gmail/v1/users/me/")
            .unwrap_or_default();
        match rest {
            "profile" => ResponseTemplate::new(200).set_body_json(json!({
                "emailAddress": mailbox.email,
                "messagesTotal": mailbox.listed.len(),
                "historyId": mailbox.profile_history_id,
            })),
            "messages" => {
                let (ids, next) = page(&mailbox.listed, &query);
                let messages: Vec<Value> = ids
                    .iter()
                    .map(|id| json!({"id": id, "threadId": format!("thread-{id}")}))
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({
                    "messages": messages,
                    "nextPageToken": next,
                    "resultSizeEstimate": mailbox.listed.len(),
                }))
            }
            "history" if mailbox.history_expired => ResponseTemplate::new(404),
            "history" => {
                let (added, next) = page(&mailbox.history, &query);
                let history: Vec<Value> = added
                    .iter()
                    .map(|(id, labels)| {
                        json!({"id": "1", "messagesAdded": [
                            {"message": {"id": id, "threadId": format!("thread-{id}"), "labelIds": labels}},
                        ]})
                    })
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({
                    "history": history,
                    "nextPageToken": next,
                    "historyId": mailbox.history_page_id,
                }))
            }
            other => match other
                .strip_prefix("messages/")
                .and_then(|id| mailbox.messages.get(id))
            {
                Some(body) => ResponseTemplate::new(200).set_body_json(body),
                None => ResponseTemplate::new(404),
            },
        }
    }
}

/// A fake Gmail on `server` for [`ME`], profile `historyId` 100.
async fn fake_gmail(server: &MockServer) -> FakeGmail {
    let gmail = FakeGmail(Arc::new(Mutex::new(Mailbox {
        email: ME.into(),
        profile_history_id: "100".into(),
        ..Default::default()
    })));
    Mock::given(method("GET"))
        .and(path_regex("^/gmail/v1/users/me/"))
        .respond_with(gmail.clone())
        .mount(server)
        .await;
    gmail
}

/// Sync settings that keep tests fast: 3 ids a page, no page delay.
fn gmail_config(server: &MockServer) -> gmail::Config {
    gmail::Config {
        base_url: server.uri(),
        page_size: 3,
        page_delay: Duration::ZERO,
        ..Default::default()
    }
}

fn google_config(server: &MockServer, gmail: gmail::Config) -> google::Config {
    google::Config {
        client_id: Some("test-client.apps.googleusercontent.com".into()),
        client_secret: Some(fake("client-secret")),
        token_key: Some(TokenKey::new(KEY)),
        auth_url: format!("{}/auth", server.uri()),
        token_url: format!("{}/token", server.uri()),
        revoke_url: format!("{}/revoke", server.uri()),
        redirect_uri: google::config::redirect_uri(8000),
        gmail,
        calendar: waypoint_api::calendar::Config {
            base_url: server.uri(),
        },
    }
}

/// Stores a connected account whose access token `access` stays valid for an hour.
async fn seed_account(pool: &PgPool, access: &str, status: &str) {
    let key = TokenKey::new(KEY);
    sqlx::query(
        "INSERT INTO google_accounts \
             (refresh_token, access_token, access_token_expires_at, scopes, status) \
         VALUES ($1, $2, now() + interval '1 hour', $3, $4)",
    )
    .bind(crypto::seal(
        &key,
        google::REFRESH_TOKEN_AAD,
        fake("refresh").as_bytes(),
    ))
    .bind(crypto::seal(
        &key,
        google::ACCESS_TOKEN_AAD,
        access.as_bytes(),
    ))
    .bind(google::SCOPES.map(String::from).to_vec())
    .bind(status)
    .execute(pool)
    .await
    .expect("seed the account");
}

/// A state row that finished just now, so the worker's startup catch-up does nothing.
async fn mark_fresh(pool: &PgPool) {
    sqlx::query("INSERT INTO gmail_sync_state (id, last_finished_at) VALUES (true, now())")
        .execute(pool)
        .await
        .expect("seed the state");
}

async fn execute(pool: &PgPool, sql: &str) {
    sqlx::query(sql).execute(pool).await.expect(sql);
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

async fn gmail_ids(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT provider_message_id FROM emails WHERE source = 'gmail' ORDER BY provider_message_id",
    )
    .fetch_all(pool)
    .await
    .expect("list emails")
}

/// `(email_address, history_id, run_mode, last_error)`.
async fn state(
    pool: &PgPool,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    sqlx::query_as(
        "SELECT email_address, history_id, run_mode, last_error FROM gmail_sync_state WHERE id",
    )
    .fetch_one(pool)
    .await
    .expect("the state row")
}

async fn pending_syncs(pool: &PgPool) -> i64 {
    count(
        pool,
        "SELECT count(*) FROM jobs WHERE kind = 'gmail_sync' AND status IN ('queued', 'running')",
    )
    .await
}

fn sum(results: &[Value], key: &str) -> i64 {
    results.iter().map(|r| r[key].as_i64().unwrap_or(0)).sum()
}

fn modes(results: &[Value]) -> Vec<&str> {
    results
        .iter()
        .map(|r| r["mode"].as_str().unwrap())
        .collect()
}

/// Whether `event` ends a run: a `gmail_sync` job that failed, stopped or had no successor.
fn ends_run(event: &Event) -> bool {
    match event {
        Event::JobFailed { job } => job.kind == jobs::GMAIL_SYNC,
        Event::JobSucceeded { job } if job.kind == jobs::GMAIL_SYNC => job
            .result
            .as_ref()
            .is_some_and(|r| r["next_job_id"].is_null()),
        _ => false,
    }
}

/// The results of the `gmail_sync` jobs that succeeded among `events`, in order.
fn page_results(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::JobSucceeded { job } if job.kind == jobs::GMAIL_SYNC => job.result.clone(),
            _ => None,
        })
        .collect()
}

/// The API with these Google settings; its worker starts (and runs catch-up) at once.
async fn client(pool: &PgPool, config: google::Config) -> Client {
    let rocket = waypoint_api::rocket(
        rocket::Config::figment(),
        pool.clone(),
        ClaudeRunner::new("claude", Duration::from_secs(1)),
        common::fast_config(),
        config,
        waypoint_api::capture::Config::default(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
}

fn host() -> Header<'static> {
    Header::new("Host", common::HOST)
}

async fn sync_now(client: &Client) -> LocalResponse<'_> {
    client.post("/gmail/sync").header(host()).dispatch().await
}

async fn sync_status(client: &Client) -> Value {
    let response = client.get("/gmail/sync").header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Ok);
    response.into_json().await.expect("json")
}

/// A running worker with Google settings, plus what a test needs to drive it.
struct Harness {
    pool: PgPool,
    bus: EventBus,
    wake: Wake,
    google: Google,
    _stop: watch::Sender<bool>,
}

impl Harness {
    /// Starts a worker. Its startup catch-up runs at once.
    fn start(pool: &PgPool, config: google::Config) -> (Self, Subscriber) {
        let bus = EventBus::new();
        let subscriber = bus.subscribe();
        let wake = Wake::default();
        let google = Google::new(config);
        let (stop, stopped) = watch::channel(false);
        Worker {
            pool: pool.clone(),
            runner: ClaudeRunner::new("claude", Duration::from_secs(1)),
            bus: bus.clone(),
            config: common::fast_config(),
            wake: wake.0.clone(),
            runtime: Default::default(),
            google: google.clone(),
            capture: waypoint_api::capture::Capturer::new(Default::default()),
        }
        .spawn(stopped);
        let harness = Self {
            pool: pool.clone(),
            bus,
            wake,
            google,
            _stop: stop,
        };
        (harness, subscriber)
    }

    async fn request(&self) -> Result<sync::Requested, sync::RequestError> {
        sync::request(
            &self.pool,
            &self.bus,
            &self.wake,
            &self.google,
            sync::Trigger::Manual,
        )
        .await
    }

    /// Requests a run and waits for it to end; returns every event seen.
    async fn run(&self, subscriber: &mut Subscriber) -> Vec<Event> {
        let requested = self.request().await.expect("request a sync");
        assert!(!requested.reused, "a run was already pending");
        let events = common::events(subscriber, TIMEOUT, ends_run).await;
        assert!(
            events.last().is_some_and(ends_run),
            "the run didn't end: {events:?}"
        );
        events
    }
}

#[sqlx::test]
async fn backfill_then_incremental(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    let stems = [
        "recruiter-outreach",
        "ashby-rejection",
        "lever-interview-invite",
        "greenhouse-application-received",
        "workday-application-update",
        "newsletter",
        "personal-email",
        "linkedin-job-alert",
    ];
    gmail.list(&stems);
    mark_fresh(&pool).await;
    seed_account(&pool, &fake("access"), "connected").await;
    let config = gmail::Config {
        max_messages: 7,
        page_delay: Duration::from_millis(100),
        ..gmail_config(&server)
    };
    let (harness, mut subscriber) = Harness::start(&pool, google_config(&server, config));

    // Backfill: pages of 3, 3 and 1 (the cap), each its own job.
    let events = harness.run(&mut subscriber).await;
    let results = page_results(&events);
    assert_eq!(modes(&results), ["backfill"; 3]);
    assert_eq!(
        results
            .iter()
            .map(|r| r["page"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(sum(&results, "inserted"), 7);
    assert_eq!(sum(&results, "seen"), 7);
    let mut expected: Vec<String> = stems[..7].iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(gmail_ids(&pool).await, expected);
    let (labels, thread): (Vec<String>, Option<String>) = sqlx::query_as(
        "SELECT e.labels, t.provider_thread_id FROM emails e \
         JOIN email_threads t ON t.id = e.thread_id WHERE e.provider_message_id = $1",
    )
    .bind("recruiter-outreach")
    .fetch_one(&pool)
    .await
    .expect("the stored email");
    assert_eq!(labels, ["INBOX"]);
    assert_eq!(thread.as_deref(), Some("thread-recruiter-outreach"));
    // Successors come from `enqueue_tx_after` with the page delay.
    let successors: Vec<Job> = sqlx::query_as(
        "SELECT id, kind, payload, status, attempts, max_attempts, run_at, last_error, result, \
         created_at, updated_at, finished_at FROM jobs WHERE kind = 'gmail_sync' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("list jobs");
    assert_eq!(successors.len(), 3);
    assert_eq!(successors[0].payload, json!({"trigger": "manual"}));
    for job in &successors[1..] {
        assert_eq!(job.payload["mode"], "backfill");
        assert_eq!(job.payload["start_history_id"], "100");
        let delay = job.run_at - job.created_at;
        assert!(delay >= chrono::Duration::milliseconds(100), "{delay}");
    }
    assert_eq!(
        state(&pool).await,
        (
            Some(ME.into()),
            Some("100".into()),
            Some("backfill".into()),
            None
        )
    );

    // Incremental from 100: only new, non-draft messages from history.list are stored.
    gmail.edit(|mailbox| {
        mailbox.profile_history_id = "105".into();
        mailbox.history_page_id = "110".into();
        // Two pages of 3: a duplicate on the first, then only skipped labels.
        mailbox.history = vec![
            ("indeed-digest".into(), vec!["INBOX".into()]),
            ("indeed-digest".into(), vec!["INBOX".into()]),
            ("forwarded-thread".into(), vec!["DRAFT".into()]),
            ("auto-reply-ooo".into(), vec!["SPAM".into()]),
        ];
        for stem in ["indeed-digest", "forwarded-thread", "auto-reply-ooo"] {
            mailbox.messages.insert(stem.into(), message(stem));
        }
    });
    let events = harness.run(&mut subscriber).await;
    let results = page_results(&events);
    assert_eq!(modes(&results), ["incremental", "incremental"]);
    assert_eq!(sum(&results, "inserted"), 1);
    assert_eq!(sum(&results, "seen"), 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM emails").await, 8);
    assert!(
        gmail_ids(&pool)
            .await
            .contains(&"indeed-digest".to_string())
    );
    let starts: Vec<String> = server
        .received_requests()
        .await
        .expect("recording is on")
        .iter()
        .filter(|r| r.url.path().ends_with("/history"))
        .map(|r| query(r)["startHistoryId"].clone())
        .collect();
    assert_eq!(starts, ["100", "100"]);
    assert_eq!(state(&pool).await.1.as_deref(), Some("110"));

    // AC11: another account restarts as a backfill and drops the old history id.
    gmail.edit(|mailbox| {
        mailbox.email = "other@example.com".into();
        mailbox.profile_history_id = "200".into();
    });
    let events = harness.run(&mut subscriber).await;
    let results = page_results(&events);
    assert_eq!(modes(&results)[0], "backfill");
    let (address, history_id, mode, _) = state(&pool).await;
    assert_eq!(address.as_deref(), Some("other@example.com"));
    assert_eq!(history_id.as_deref(), Some("200"));
    assert_eq!(mode.as_deref(), Some("backfill"));
}

#[sqlx::test]
async fn expired_history_resyncs_bounded(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&[
        "recruiter-outreach",
        "ashby-rejection",
        "lever-interview-invite",
        "greenhouse-application-received",
        "workday-application-update",
        "newsletter",
        "personal-email",
    ]);
    mark_fresh(&pool).await;
    seed_account(&pool, &fake("access"), "connected").await;
    let config = gmail::Config {
        max_messages: 5,
        ..gmail_config(&server)
    };
    let (harness, mut subscriber) = Harness::start(&pool, google_config(&server, config));
    harness.run(&mut subscriber).await;
    let before: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, provider_message_id FROM emails ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("list emails");
    assert_eq!(before.len(), 5);

    // Two newer messages arrive, and the stored history has expired.
    gmail.edit(|mailbox| {
        mailbox.history_expired = true;
        mailbox.profile_history_id = "300".into();
        mailbox.history_page_id = "999".into();
        for stem in ["indeed-digest", "forwarded-thread"] {
            mailbox.listed.insert(0, stem.into());
            mailbox.messages.insert(stem.into(), message(stem));
        }
    });
    let events = harness.run(&mut subscriber).await;
    let results = page_results(&events);
    assert_eq!(modes(&results), ["resync", "resync"]);
    assert_eq!(sum(&results, "seen"), 5, "bounded by max_messages");
    assert_eq!(sum(&results, "inserted"), 2);
    assert_eq!(sum(&results, "updated"), 3);
    assert_eq!(count(&pool, "SELECT count(*) FROM emails").await, 7);
    let kept: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, provider_message_id FROM emails WHERE id <= $1 ORDER BY id")
            .bind(before.last().unwrap().0)
            .fetch_all(&pool)
            .await
            .expect("list emails");
    assert_eq!(kept, before, "existing rows keep their ids");
    let (_, history_id, mode, _) = state(&pool).await;
    assert_eq!(history_id.as_deref(), Some("300"), "from getProfile");
    assert_eq!(mode.as_deref(), Some("resync"));
}

#[sqlx::test]
async fn oversized_message_is_skipped(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    // 323 and 459 bytes fit under 1000; 1194 doesn't.
    gmail.list(&["personal-email", "newsletter", "gmail-reply-quoted"]);
    gmail.edit(|mailbox| {
        // A small message inside an HTTP body past the read limit.
        let mut huge = message("recruiter-outreach");
        huge["id"] = json!("huge-body");
        huge["padding"] = json!("x".repeat(80 * 1024));
        mailbox.listed.push("huge-body".into());
        mailbox.messages.insert("huge-body".into(), huge);
        // Deleted since it was listed.
        mailbox.listed.push("gone".into());
        // Answers with another message's id.
        mailbox.listed.push("swapped".into());
        mailbox
            .messages
            .insert("swapped".into(), message("newsletter"));
    });
    mark_fresh(&pool).await;
    seed_account(&pool, &fake("access"), "connected").await;
    let config = gmail::Config {
        max_message_bytes: 1000,
        ..gmail_config(&server)
    };
    let (harness, mut subscriber) = Harness::start(&pool, google_config(&server, config));
    let results = page_results(&harness.run(&mut subscriber).await);
    assert_eq!(sum(&results, "oversized"), 2);
    assert_eq!(sum(&results, "vanished"), 1);
    assert_eq!(sum(&results, "failed"), 1);
    assert_eq!(sum(&results, "inserted"), 2);
    assert_eq!(sum(&results, "seen"), 6);
    assert_eq!(gmail_ids(&pool).await, ["newsletter", "personal-email"]);
}

#[sqlx::test]
async fn resync_is_idempotent(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&[
        "recruiter-outreach",
        "gmail-reply-quoted",
        "forwarded-thread",
        "recruiter-outlook-reply",
        "newsletter",
    ]);
    mark_fresh(&pool).await;
    seed_account(&pool, &fake("access"), "connected").await;
    let (harness, mut subscriber) =
        Harness::start(&pool, google_config(&server, gmail_config(&server)));
    let first = page_results(&harness.run(&mut subscriber).await);
    assert_eq!(sum(&first, "inserted"), 5);
    let emails = count(&pool, "SELECT count(*) FROM emails").await;
    let threads = count(&pool, "SELECT count(*) FROM email_threads").await;

    gmail.edit(|mailbox| mailbox.history_expired = true);
    let second = page_results(&harness.run(&mut subscriber).await);
    assert_eq!(modes(&second), ["resync", "resync"]);
    assert_eq!(sum(&second, "inserted"), 0);
    assert_eq!(sum(&second, "updated"), 5);
    assert_eq!(count(&pool, "SELECT count(*) FROM emails").await, emails);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM email_threads").await,
        threads
    );
}

#[sqlx::test]
async fn sync_now_reuses_pending_and_chain_ends(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&[
        "recruiter-outreach",
        "ashby-rejection",
        "lever-interview-invite",
        "greenhouse-application-received",
        "newsletter",
        "personal-email",
        "indeed-digest",
    ]);
    seed_account(&pool, &fake("access"), "connected").await;
    let bus = EventBus::new();
    let wake = Wake::default();
    let google = Google::new(google_config(&server, gmail_config(&server)));
    let request = || sync::request(&pool, &bus, &wake, &google, sync::Trigger::Manual);

    // No worker yet: the first request enqueues, the next ones reuse it.
    let first = request().await.expect("request");
    assert!(!first.reused);
    let again = request().await.expect("request");
    assert!(again.reused);
    assert_eq!(again.job.id, first.job.id);

    // A queued job waiting out a delay is pulled forward to now.
    execute(&pool, "UPDATE jobs SET run_at = now() + interval '1 hour'").await;
    let pulled = request().await.expect("request");
    assert!(pulled.reused);
    assert!(
        pulled.job.run_at <= chrono::Utc::now(),
        "{}",
        pulled.job.run_at
    );

    // A running one is returned as is.
    execute(&pool, "UPDATE jobs SET status = 'running'").await;
    let running = request().await.expect("request");
    assert!(running.reused);
    assert_eq!(running.job.status, "running");
    assert_eq!(count(&pool, "SELECT count(*) FROM jobs").await, 1);

    // The worker requeues it as an orphan, runs the chain, and the chain ends.
    let (harness, mut subscriber) =
        Harness::start(&pool, google_config(&server, gmail_config(&server)));
    let events = common::events(&mut subscriber, TIMEOUT, ends_run).await;
    let results = page_results(&events);
    assert_eq!(results.len(), 3, "{results:?}");
    assert_eq!(sum(&results, "inserted"), 7);
    assert_eq!(pending_syncs(&pool).await, 0);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM jobs WHERE kind = 'gmail_sync'").await,
        3
    );
    // And a new request starts a new run.
    assert!(!harness.request().await.expect("request").reused);
}

#[sqlx::test]
async fn no_sync_without_google(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&[
        "recruiter-outreach",
        "ashby-rejection",
        "lever-interview-invite",
        "greenhouse-application-received",
        "newsletter",
    ]);
    let bus = EventBus::new();
    let configured = Google::new(google_config(&server, gmail_config(&server)));
    let unconfigured = Google::new(google::Config {
        gmail: gmail_config(&server),
        ..Default::default()
    });
    let refused = |google: &Google| {
        let (pool, bus) = (pool.clone(), bus.clone());
        let google = google.clone();
        async move {
            let requested = sync::request(
                &pool,
                &bus,
                &Wake::default(),
                &google,
                sync::Trigger::Manual,
            )
            .await;
            assert!(
                matches!(requested, Err(sync::RequestError::NotConnected)),
                "{requested:?}"
            );
            assert_eq!(
                sync::catch_up(&pool, &bus, &google)
                    .await
                    .expect("catch up"),
                None
            );
            let client = client(&pool, google.config.clone()).await;
            let response = sync_now(&client).await;
            assert_eq!(response.status(), Status::Conflict);
            assert_eq!(response.content_type(), Some(ContentType::Plain));
            assert_eq!(
                response.into_string().await.as_deref(),
                Some("Connect Google before syncing Gmail.")
            );
        }
    };

    // Not configured (even with an account), disconnected, and reconnect required.
    seed_account(&pool, &fake("access"), "connected").await;
    refused(&unconfigured).await;
    execute(&pool, "DELETE FROM google_accounts").await;
    refused(&configured).await;
    seed_account(&pool, &fake("access"), "reconnect_required").await;
    refused(&configured).await;
    assert_eq!(count(&pool, "SELECT count(*) FROM jobs").await, 0);

    // The account is removed mid-run: the next page stops with no successor.
    execute(&pool, "DELETE FROM google_accounts").await;
    mark_fresh(&pool).await;
    seed_account(&pool, &fake("access"), "connected").await;
    let config = gmail::Config {
        page_size: 2,
        page_delay: Duration::from_millis(300),
        ..gmail_config(&server)
    };
    let (harness, mut subscriber) = Harness::start(&pool, google_config(&server, config));
    harness.request().await.expect("request");
    let first = common::events(
        &mut subscriber,
        TIMEOUT,
        |event| matches!(event, Event::JobSucceeded { job } if job.kind == jobs::GMAIL_SYNC),
    )
    .await;
    assert!(page_results(&first)[0]["next_job_id"].is_i64());
    execute(&pool, "DELETE FROM google_accounts").await;
    let rest = common::events(&mut subscriber, TIMEOUT, ends_run).await;
    let stopped = page_results(&rest);
    assert_eq!(
        stopped.last().unwrap()["stopped"],
        "no Google account is connected"
    );
    assert_eq!(pending_syncs(&pool).await, 0);
    let (_, history_id, _, last_error) = state(&pool).await;
    assert_eq!(history_id, None, "an unfinished run stores no history id");
    assert_eq!(
        last_error.as_deref(),
        Some("no Google account is connected")
    );
}

#[sqlx::test]
async fn startup_catch_up_when_stale(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&["recruiter-outreach", "newsletter"]);
    seed_account(&pool, &fake("access"), "connected").await;
    let config = || google_config(&server, gmail_config(&server));
    let startup_jobs = || {
        count(
            &pool,
            "SELECT count(*) FROM jobs WHERE payload->>'trigger' = 'startup'",
        )
    };

    // Never synced, and synced an hour ago: one catch-up run each.
    for state in [
        None,
        Some(
            "INSERT INTO gmail_sync_state (id, last_finished_at) VALUES (true, now() - interval '1 hour')",
        ),
    ] {
        execute(&pool, "DELETE FROM jobs").await;
        execute(&pool, "DELETE FROM gmail_sync_state").await;
        if let Some(sql) = state {
            execute(&pool, sql).await;
        }
        let (_harness, mut subscriber) = Harness::start(&pool, config());
        let events = common::events(&mut subscriber, TIMEOUT, ends_run).await;
        assert!(events.last().is_some_and(ends_run), "{state:?}: {events:?}");
        assert_eq!(startup_jobs().await, 1, "{state:?}");
        assert_eq!(
            count(&pool, "SELECT count(*) FROM jobs WHERE kind = 'gmail_sync'").await,
            1,
            "{state:?}"
        );
    }

    // Synced just now: nothing.
    execute(&pool, "DELETE FROM jobs").await;
    execute(
        &pool,
        "UPDATE gmail_sync_state SET last_finished_at = now()",
    )
    .await;
    let (_harness, mut subscriber) = Harness::start(&pool, config());
    let events = common::events(&mut subscriber, Duration::from_millis(500), |_| false).await;
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(count(&pool, "SELECT count(*) FROM jobs").await, 0);
    drop(_harness);

    // A stale state with an orphaned page: the requeued orphan is the run.
    execute(&pool, "UPDATE gmail_sync_state SET last_finished_at = NULL").await;
    execute(
        &pool,
        "INSERT INTO jobs (kind, payload, status, max_attempts) \
         VALUES ('gmail_sync', '{\"trigger\": \"manual\"}', 'running', 5)",
    )
    .await;
    let (_harness, mut subscriber) = Harness::start(&pool, config());
    let events = common::events(&mut subscriber, TIMEOUT, ends_run).await;
    assert!(events.last().is_some_and(ends_run), "{events:?}");
    assert_eq!(startup_jobs().await, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM jobs").await, 1);
}

#[sqlx::test]
async fn rate_limit_and_unauthorized_retry(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&["recruiter-outreach", "newsletter"]);
    // The first list answers 429 with Retry-After: 1.
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/messages"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    mark_fresh(&pool).await;
    seed_account(&pool, &fake("access"), "connected").await;
    let (harness, mut subscriber) =
        Harness::start(&pool, google_config(&server, gmail_config(&server)));

    let events = harness.run(&mut subscriber).await;
    let retried: Vec<&Job> = events
        .iter()
        .filter_map(|event| match event {
            Event::JobRetrying { job } => Some(job),
            _ => None,
        })
        .collect();
    assert_eq!(retried.len(), 1, "{events:?}");
    let wait = retried[0].run_at - retried[0].updated_at;
    assert!(wait >= chrono::Duration::seconds(1), "{wait}");
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::QueuePaused { .. })),
        "a Gmail rate limit doesn't pause the queue"
    );
    assert_eq!(sum(&page_results(&events), "inserted"), 2);

    // A refused token is expired, refreshed and the retry succeeds.
    server.reset().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&["recruiter-outreach", "newsletter", "personal-email"]);
    let stale = format!("Bearer {}", fake("access"));
    Mock::given(method("GET"))
        .and(header("authorization", stale.as_str()))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": fake("fresh-access"),
            "expires_in": 3599,
            "token_type": "Bearer",
        })))
        .expect(1)
        .mount(&server)
        .await;
    execute(&pool, "DELETE FROM gmail_sync_state").await;
    let events = harness.run(&mut subscriber).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobRetrying { .. })),
        "{events:?}"
    );
    assert_eq!(sum(&page_results(&events), "seen"), 3);
    let fresh = format!("Bearer {}", fake("fresh-access"));
    let requests = server.received_requests().await.expect("recording is on");
    let gets: Vec<&str> = requests
        .iter()
        .filter(|r| r.method.as_str() == "GET")
        .filter_map(|r| r.headers.get("authorization")?.to_str().ok())
        .collect();
    assert_eq!(gets[0], stale);
    assert!(gets[1..].iter().all(|auth| *auth == fresh), "{gets:?}");
    server.verify().await;
}

#[sqlx::test]
async fn status_and_sync_now_routes(pool: PgPool) {
    let server = MockServer::start().await;
    let gmail = fake_gmail(&server).await;
    gmail.list(&[
        "recruiter-outreach",
        "newsletter",
        "personal-email",
        "ashby-rejection",
    ]);
    mark_fresh(&pool).await;
    let client = client(&pool, google_config(&server, gmail_config(&server))).await;
    seed_account(&pool, &fake("access"), "connected").await;
    let bus = client.rocket().state::<EventBus>().expect("the bus");
    let mut subscriber = bus.subscribe();

    let before = sync_status(&client).await;
    assert_eq!(before["activity"], "idle");
    assert_eq!(before["next_run_at"], Value::Null);
    assert_eq!(before["mode"], Value::Null);
    assert_eq!(before["last_error"], Value::Null);
    assert_eq!(before["emails_stored"], 0);
    assert_eq!(
        before["run"],
        json!({"seen": 0, "inserted": 0, "updated": 0, "oversized": 0, "failed": 0, "vanished": 0})
    );

    // A page waiting out a rate limit shows as queued until its run_at.
    let waiting: (i64, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO jobs (kind, payload, max_attempts, run_at) \
         VALUES ('gmail_sync', '{\"trigger\": \"manual\"}', 5, now() + interval '1 hour') \
         RETURNING id, run_at",
    )
    .fetch_one(&pool)
    .await
    .expect("queue a delayed page");
    let queued = sync_status(&client).await;
    assert_eq!(queued["activity"], "queued");
    let next: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(queued["next_run_at"].clone()).expect("a time");
    assert_eq!(next, waiting.1);

    // Sync now reuses it, pulled forward, and the worker runs it.
    let response = sync_now(&client).await;
    assert_eq!(response.status(), Status::Accepted);
    let body: Value = response.into_json().await.expect("json");
    assert_eq!(body["reused"], true);
    assert_eq!(body["job"]["id"], waiting.0);
    assert_eq!(body["job"]["kind"], "gmail_sync");
    let events = common::events(&mut subscriber, TIMEOUT, ends_run).await;
    assert!(events.last().is_some_and(ends_run), "{events:?}");

    let after = sync_status(&client).await;
    assert_eq!(after["activity"], "idle");
    assert_eq!(after["mode"], "backfill");
    assert_eq!(after["emails_stored"], 4);
    assert_eq!(after["run"]["inserted"], 4);
    assert_eq!(after["run"]["seen"], 4);
    assert!(after["last_finished_at"].is_string());
    assert!(after["run_started_at"].is_string());
    assert_eq!(after["last_error"], Value::Null);

    // A new run: 202 with a fresh job.
    let response = sync_now(&client).await;
    assert_eq!(response.status(), Status::Accepted);
    let body: Value = response.into_json().await.expect("json");
    assert_eq!(body["reused"], false);
    assert_ne!(body["job"]["id"], waiting.0);
    let events = common::events(&mut subscriber, TIMEOUT, ends_run).await;
    assert!(events.last().is_some_and(ends_run), "{events:?}");

    // A running page and a recorded error.
    execute(
        &pool,
        "INSERT INTO jobs (kind, payload, status, max_attempts) \
         VALUES ('gmail_sync', '{}', 'running', 5)",
    )
    .await;
    execute(
        &pool,
        "UPDATE gmail_sync_state SET last_error = 'Gmail answered HTTP 500', last_error_at = now()",
    )
    .await;
    let running = sync_status(&client).await;
    assert_eq!(running["activity"], "running");
    assert_eq!(running["next_run_at"], Value::Null);
    assert_eq!(running["mode"], "incremental");
    assert_eq!(running["last_error"], "Gmail answered HTTP 500");
    assert!(running["last_error_at"].is_string());
}

#[sqlx::test]
async fn sync_now_rejects_cross_site_and_forms(pool: PgPool) {
    let server = MockServer::start().await;
    fake_gmail(&server).await;
    mark_fresh(&pool).await;
    let client = client(&pool, google_config(&server, gmail_config(&server))).await;
    seed_account(&pool, &fake("access"), "connected").await;

    let cross_site = client
        .post("/gmail/sync")
        .header(host())
        .header(Header::new("Sec-Fetch-Site", "cross-site"))
        .dispatch()
        .await;
    assert_eq!(cross_site.status(), Status::Forbidden);
    let form = client
        .post("/gmail/sync")
        .header(host())
        .header(ContentType::Form)
        .body("a=b")
        .dispatch()
        .await;
    assert_eq!(form.status(), Status::Forbidden);
    assert_eq!(count(&pool, "SELECT count(*) FROM jobs").await, 0);

    // The web app's own request (another localhost port) passes.
    let same_site = client
        .post("/gmail/sync")
        .header(host())
        .header(Header::new("Sec-Fetch-Site", "same-site"))
        .dispatch()
        .await;
    assert_eq!(same_site.status(), Status::Accepted);
}
