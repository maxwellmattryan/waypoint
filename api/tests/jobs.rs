//! The job queue and worker against `fake-claude.sh`; the real CLI is never called here.

mod common;

use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
use waypoint_api::events::{Event, EventBus, Subscriber};
use waypoint_api::jobs::{self, Job, Wake, Worker};

const TIMEOUT: Duration = Duration::from_secs(10);

/// A worker running against the fake in `mode`, plus what a test needs to drive it.
struct Harness {
    bus: EventBus,
    wake: Wake,
    pool: PgPool,
    _fake: common::Fake,
    _stop: watch::Sender<bool>,
}

impl Harness {
    /// Starts a worker. Subscribe to `bus` before enqueueing to see every event.
    fn start(pool: &PgPool, mode: &str, config: jobs::Config) -> (Self, Subscriber) {
        let fake = common::fake(mode, TIMEOUT);
        let bus = EventBus::new();
        let subscriber = bus.subscribe();
        let wake = Wake::default();
        let (stop, stopped) = watch::channel(false);
        Worker {
            pool: pool.clone(),
            runner: fake.runner.clone(),
            bus: bus.clone(),
            config,
            wake: wake.0.clone(),
        }
        .spawn(stopped);
        let harness = Self {
            bus,
            wake,
            pool: pool.clone(),
            _fake: fake,
            _stop: stop,
        };
        (harness, subscriber)
    }

    async fn enqueue(&self, kind: &str, payload: Value, max_attempts: i32) -> Job {
        jobs::enqueue_with(
            &self.pool,
            &self.bus,
            &self.wake,
            kind,
            payload,
            max_attempts,
        )
        .await
        .expect("enqueue")
    }

    async fn test_prompt(&self, max_attempts: i32) -> Job {
        self.enqueue(jobs::TEST_PROMPT, json!({"prompt": "ping"}), max_attempts)
            .await
    }
}

fn is_finished(event: &Event, id: i64) -> bool {
    matches!(event, Event::JobSucceeded { job } | Event::JobFailed { job } if job.id == id)
}

/// The names of the `job.*` events about job `id`, in order.
fn job_events(events: &[Event], id: i64) -> Vec<&'static str> {
    events
        .iter()
        .filter(|event| event.job().is_some_and(|job| job.id == id))
        .map(Event::name)
        .collect()
}

async fn claude_calls(pool: &PgPool) -> Vec<(i64, String)> {
    sqlx::query_as("SELECT id, purpose FROM claude_calls ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read claude_calls")
}

#[sqlx::test]
async fn claim_skips_locked_rows(pool: PgPool) {
    let job = jobs::enqueue(
        &pool,
        &EventBus::new(),
        &Wake::default(),
        jobs::TEST_PROMPT,
        json!({"prompt": "ping"}),
    )
    .await
    .expect("enqueue");

    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SELECT id FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(job.id)
        .execute(&mut *tx)
        .await
        .expect("lock the row");
    let claimed = tokio::time::timeout(Duration::from_secs(2), jobs::claim(&pool))
        .await
        .expect("claim does not block on a locked row")
        .expect("claim");
    assert_eq!(claimed, None);
    tx.commit().await.expect("commit");

    let claimed = jobs::claim(&pool)
        .await
        .expect("claim")
        .expect("the row is free");
    assert_eq!(claimed.id, job.id);
    assert_eq!(claimed.status, "running");
    assert_eq!(claimed.attempts, 1);
}

#[sqlx::test]
async fn test_prompt_job_succeeds_and_emits_events(pool: PgPool) {
    let (harness, mut subscriber) = Harness::start(&pool, "success", common::fast_config());
    let job = harness.test_prompt(3).await;
    let events = common::events(&mut subscriber, TIMEOUT, |e| is_finished(e, job.id)).await;

    assert_eq!(
        job_events(&events, job.id),
        ["job.queued", "job.started", "job.succeeded"]
    );
    let done = jobs::get(&pool, job.id).await.expect("get").expect("job");
    assert_eq!(done.status, "succeeded");
    assert_eq!(done.attempts, 1);
    assert!(done.finished_at.is_some());
    let result = done.result.expect("result");
    assert_eq!(result["text"], "pong");
    assert_eq!(result["model"], "claude-haiku-5-5");
    assert_eq!(result["cost_usd"], 0.00067409);
    assert!(result["duration_ms"].is_u64(), "{result}");

    let calls = claude_calls(&pool).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(result["log_id"], calls[0].0);
    assert_eq!(calls[0].1, "job.test_prompt");
}

#[sqlx::test]
async fn worker_respects_concurrency_limit(pool: PgPool) {
    let (harness, mut subscriber) = Harness::start(&pool, "slow", common::fast_config());
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(harness.test_prompt(3).await.id);
    }

    let mut running = 0i32;
    let mut peak = 0;
    let mut succeeded = 0;
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while succeeded < 3 {
        let event = tokio::time::timeout_at(deadline, subscriber.next())
            .await
            .expect("all jobs finish in time")
            .expect("bus open");
        match event {
            Event::JobStarted { .. } => running += 1,
            Event::JobSucceeded { .. } => {
                running -= 1;
                succeeded += 1;
            }
            Event::JobFailed { .. } | Event::JobRetrying { .. } | Event::JobDeferred { .. } => {
                panic!("unexpected {event:?}")
            }
            _ => {}
        }
        peak = peak.max(running);
    }
    assert_eq!(peak, 2);
    for id in ids {
        let job = jobs::get(&pool, id).await.expect("get").expect("job");
        assert_eq!(job.status, "succeeded");
    }
}

#[sqlx::test]
async fn failing_job_retries_with_backoff_then_fails(pool: PgPool) {
    let (harness, mut subscriber) = Harness::start(&pool, "exit1", common::fast_config());
    let job = harness.test_prompt(2).await;
    let events = common::events(&mut subscriber, TIMEOUT, |e| is_finished(e, job.id)).await;

    assert_eq!(
        job_events(&events, job.id),
        [
            "job.queued",
            "job.started",
            "job.retrying",
            "job.started",
            "job.failed"
        ]
    );
    let retrying = events
        .iter()
        .find_map(|event| match event {
            Event::JobRetrying { job } => Some(job),
            _ => None,
        })
        .expect("a retrying event");
    assert_eq!(retrying.status, "queued");
    assert!(
        retrying.run_at - retrying.updated_at >= chrono::Duration::milliseconds(50),
        "run_at {} is less than 50ms after the failure at {}",
        retrying.run_at,
        retrying.updated_at
    );

    let failed = jobs::get(&pool, job.id).await.expect("get").expect("job");
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.attempts, 2);
    assert!(failed.finished_at.is_some());
    let error = failed.last_error.expect("last_error");
    assert!(error.contains("something broke"), "{error}");
    assert_eq!(claude_calls(&pool).await.len(), 2);
}

#[sqlx::test]
async fn usage_limit_defers_job_and_pauses_queue(pool: PgPool) {
    let (harness, mut subscriber) = Harness::start(&pool, "usage_limit", common::fast_config());
    let job = harness.test_prompt(3).await;
    let events = common::events(&mut subscriber, TIMEOUT, |e| {
        matches!(e, Event::QueuePaused { .. })
    })
    .await;
    let names: Vec<_> = events.iter().map(Event::name).collect();
    assert_eq!(
        names,
        ["job.queued", "job.started", "job.deferred", "queue.paused"]
    );

    let Event::JobDeferred { job: deferred } = &events[2] else {
        panic!("expected job.deferred, got {:?}", events[2]);
    };
    assert_eq!(deferred.id, job.id);
    assert_eq!(deferred.status, "queued");
    assert_eq!(
        deferred.attempts, 0,
        "a usage limit does not use up an attempt"
    );
    assert!(
        deferred.run_at - deferred.updated_at >= chrono::Duration::seconds(60),
        "{deferred:?}"
    );
    let error = deferred.last_error.as_deref().expect("last_error");
    assert!(error.contains("hit your limit"), "{error}");

    let Event::QueuePaused { until } = &events[3] else {
        panic!("expected queue.paused, got {:?}", events[3]);
    };
    let expected = Utc::now() + chrono::Duration::seconds(60);
    assert!(
        (*until - expected).num_seconds().abs() <= 5,
        "until {until} is not about {expected}"
    );

    let second = harness.test_prompt(3).await;
    let later = common::events(&mut subscriber, Duration::from_millis(500), |_| false).await;
    assert!(
        !later
            .iter()
            .any(|event| matches!(event, Event::JobStarted { .. })),
        "a job started during the pause: {later:?}"
    );
    let second = jobs::get(&pool, second.id)
        .await
        .expect("get")
        .expect("job");
    assert_eq!((second.status.as_str(), second.attempts), ("queued", 0));
}

#[sqlx::test]
async fn queue_resumes_after_pause(pool: PgPool) {
    let config = jobs::Config {
        usage_limit_pause: Duration::from_millis(300),
        ..common::fast_config()
    };
    let (harness, mut subscriber) = Harness::start(&pool, "usage_limit", config);
    let job = harness.test_prompt(3).await;

    let mut started = 0;
    let events = common::events(&mut subscriber, TIMEOUT, |event| {
        // The fake keeps hitting the limit, so wait for the start after the resume.
        matches!(event, Event::JobStarted { .. }) && {
            started += 1;
            started == 2
        }
    })
    .await;
    let names: Vec<_> = events.iter().map(Event::name).collect();
    assert_eq!(
        names,
        [
            "job.queued",
            "job.started",
            "job.deferred",
            "queue.paused",
            "queue.resumed",
            "job.started"
        ]
    );
    assert_eq!(events[5].job().map(|j| j.id), Some(job.id));
}

#[sqlx::test]
async fn unknown_kind_and_bad_payload_fail_without_retry(pool: PgPool) {
    let (harness, mut subscriber) = Harness::start(&pool, "success", common::fast_config());
    let unknown = harness.enqueue("no_such_kind", json!({}), 3).await;
    let bad = harness.enqueue(jobs::TEST_PROMPT, json!({}), 3).await;
    let mut failed = 0;
    let events = common::events(&mut subscriber, TIMEOUT, |event| {
        if is_finished(event, unknown.id) || is_finished(event, bad.id) {
            failed += 1;
        }
        failed == 2
    })
    .await;

    for (job, needle) in [(&unknown, "no_such_kind"), (&bad, "prompt")] {
        assert_eq!(
            job_events(&events, job.id),
            ["job.queued", "job.started", "job.failed"]
        );
        let row = jobs::get(&pool, job.id).await.expect("get").expect("job");
        assert_eq!((row.status.as_str(), row.attempts), ("failed", 1));
        let error = row.last_error.expect("last_error");
        assert!(error.contains(needle), "{error}");
    }
    assert!(
        claude_calls(&pool).await.is_empty(),
        "no claude call was made"
    );
}

#[sqlx::test]
async fn orphaned_running_jobs_are_requeued_on_start(pool: PgPool) {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO jobs (kind, payload, status, attempts) \
         VALUES ('test_prompt', '{\"prompt\": \"ping\"}', 'running', 1) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert an orphan");

    let (_harness, mut subscriber) = Harness::start(&pool, "success", common::fast_config());
    let events = common::events(&mut subscriber, TIMEOUT, |e| is_finished(e, id)).await;

    assert_eq!(job_events(&events, id), ["job.started", "job.succeeded"]);
    let job = jobs::get(&pool, id).await.expect("get").expect("job");
    assert_eq!(job.status, "succeeded");
    assert_eq!(job.result.expect("result")["text"], "pong");
}
