//! The in-process worker: claims ready jobs up to the concurrency limit, runs each one's
//! handler, records the outcome and publishes it. A usage limit pauses the whole queue.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

use super::{AGENT_RUN, CAPTURE_EXTRACT, Config, Job, TEST_PROMPT};
use crate::capture::{self, llm::LlmError};
use crate::claude::{ClaudeError, ClaudeRequest, ClaudeRunner};
use crate::events::{Event, EventBus};
use crate::runs;

/// The delay before retry number `attempt` (1-based): `base * 2^(attempt-1)`, capped at
/// `cap`. It saturates instead of overflowing.
pub fn backoff(attempt: i32, base: Duration, cap: Duration) -> Duration {
    let doublings = u32::try_from(attempt.saturating_sub(1)).unwrap_or(0);
    let factor = 2u32.checked_pow(doublings).unwrap_or(u32::MAX);
    base.checked_mul(factor).unwrap_or(cap).min(cap)
}

/// Runs queued jobs. Build it with the shared pool, runner and bus, then [`Worker::spawn`].
pub struct Worker {
    pub pool: PgPool,
    pub runner: ClaudeRunner,
    pub bus: EventBus,
    pub config: Config,
    /// Notified by `enqueue`, so a new job starts without waiting for the next poll.
    pub wake: Arc<Notify>,
    /// The workflows and tools `agent_run` jobs can use.
    pub runtime: runs::Runtime,
}

/// What a handler decided about one attempt.
enum JobOutcome {
    /// Succeeded with this `result`.
    Done(Value),
    /// Failed in a way another attempt might fix.
    Retry(String),
    /// The subscription's usage limit was hit.
    UsageLimit(String),
    /// Failed for good; no retry.
    Fail(String),
    /// The kill switch stopped the run; the job waits in the queue with its attempt back.
    Paused,
}

/// What a running job's task needs; cheap to clone.
#[derive(Clone)]
struct Context {
    pool: PgPool,
    runner: ClaudeRunner,
    bus: EventBus,
    config: Config,
    runtime: runs::Runtime,
    /// Tells the loop to pause until this time.
    pause: mpsc::UnboundedSender<DateTime<Utc>>,
}

impl Worker {
    /// Requeues orphaned jobs, then runs the loop until `shutdown` turns `true` (or its
    /// sender is dropped). On shutdown the in-flight jobs are dropped, which kills their
    /// `claude` processes; their rows stay `running` until the next start requeues them.
    pub fn spawn(self, shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
        tokio::spawn(self.run(shutdown))
    }

    async fn run(self, mut shutdown: watch::Receiver<bool>) {
        match super::requeue_orphans(&self.pool).await {
            Ok(0) => {}
            Ok(n) => rocket::info!("jobs: requeued {n} job(s) left running by the last run"),
            Err(err) => rocket::warn!("jobs: cannot requeue orphaned jobs: {err}"),
        }

        let (pause_tx, mut pause_rx) = mpsc::unbounded_channel();
        let ctx = Context {
            pool: self.pool.clone(),
            runner: self.runner.clone(),
            bus: self.bus.clone(),
            config: self.config.clone(),
            runtime: self.runtime.clone(),
            pause: pause_tx,
        };
        let poll = self.config.poll_interval;
        let permits = Arc::new(Semaphore::new(self.config.concurrency.max(1)));
        let mut tasks = JoinSet::new();
        let mut paused_until: Option<DateTime<Utc>> = None;

        loop {
            while let Ok(until) = pause_rx.try_recv() {
                self.extend_pause(&mut paused_until, until);
            }
            while let Some(done) = tasks.try_join_next() {
                if let Err(err) = done {
                    rocket::warn!("jobs: a job task ended abnormally: {err}");
                }
            }

            if let Some(until) = paused_until {
                let left = (until - Utc::now()).to_std().unwrap_or(Duration::ZERO);
                if left.is_zero() {
                    paused_until = None;
                    self.bus.publish(Event::QueueResumed);
                    continue;
                }
                tokio::select! {
                    () = tokio::time::sleep(left) => {}
                    Some(until) = pause_rx.recv() => self.extend_pause(&mut paused_until, until),
                    () = stopped(&mut shutdown) => break,
                }
                continue;
            }

            // Take a permit before claiming, so a claimed row always starts at once.
            let permit = tokio::select! {
                permit = permits.clone().acquire_owned() => {
                    permit.expect("the semaphore is never closed")
                }
                Some(until) = pause_rx.recv() => {
                    self.extend_pause(&mut paused_until, until);
                    continue;
                }
                () = stopped(&mut shutdown) => break,
            };
            if *shutdown.borrow() {
                break;
            }

            match super::claim(&self.pool).await {
                Ok(Some(job)) => {
                    self.bus.publish(Event::JobStarted { job: job.clone() });
                    tasks.spawn(run_job(ctx.clone(), job, permit));
                }
                Ok(None) => {
                    drop(permit);
                    tokio::select! {
                        () = tokio::time::sleep(poll) => {}
                        () = self.wake.notified() => {}
                        Some(until) = pause_rx.recv() => self.extend_pause(&mut paused_until, until),
                        () = stopped(&mut shutdown) => break,
                    }
                }
                Err(err) => {
                    drop(permit);
                    rocket::warn!("jobs: cannot claim a job: {err}");
                    tokio::select! {
                        () = tokio::time::sleep(poll) => {}
                        () = stopped(&mut shutdown) => break,
                    }
                }
            }
        }

        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    /// Pauses the queue until `until`, unless it is already paused for longer, and
    /// announces the new end.
    fn extend_pause(&self, paused_until: &mut Option<DateTime<Utc>>, until: DateTime<Utc>) {
        if paused_until.is_none_or(|current| until > current) {
            *paused_until = Some(until);
            self.bus.publish(Event::QueuePaused { until });
        }
    }
}

/// Resolves once shutdown is requested or the sender is gone.
async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|stop| *stop).await;
}

/// Runs one claimed job and records its outcome. Holds `_permit` until it is done.
async fn run_job(ctx: Context, job: Job, _permit: OwnedSemaphorePermit) {
    let outcome = dispatch(&ctx, &job).await;
    let mut pause_until = None;
    loop {
        let written = match &outcome {
            JobOutcome::Done(result) => super::succeed(&ctx.pool, job.id, result)
                .await
                .map(|job| Event::JobSucceeded { job }),
            JobOutcome::Retry(error) if job.attempts < job.max_attempts => {
                let delay = backoff(job.attempts, ctx.config.retry_base, ctx.config.retry_cap);
                super::retry(&ctx.pool, job.id, error, delay)
                    .await
                    .map(|job| Event::JobRetrying { job })
            }
            JobOutcome::Retry(error) | JobOutcome::Fail(error) => {
                super::fail(&ctx.pool, job.id, error)
                    .await
                    .map(|job| Event::JobFailed { job })
            }
            JobOutcome::UsageLimit(error) => {
                let pause = ctx.config.usage_limit_pause;
                pause_until = chrono::Duration::from_std(pause)
                    .ok()
                    .and_then(|pause| Utc::now().checked_add_signed(pause));
                super::defer(&ctx.pool, job.id, error, pause)
                    .await
                    .map(|job| Event::JobDeferred { job })
            }
            // `claim` skips it until the kill switch is off; the queue itself runs on.
            JobOutcome::Paused => {
                super::defer(&ctx.pool, job.id, "agents are paused", Duration::ZERO)
                    .await
                    .map(|job| Event::JobDeferred { job })
            }
        };
        match written {
            Ok(event) => {
                ctx.bus.publish(event);
                break;
            }
            Err(sqlx::Error::RowNotFound) => {
                // Deleted under it, by `DELETE /data`; retrying would loop forever.
                rocket::warn!(
                    "jobs: job {} no longer exists; dropping its outcome",
                    job.id
                );
                break;
            }
            Err(err) => {
                rocket::warn!("jobs: cannot record the outcome of job {}: {err}", job.id);
                tokio::time::sleep(ctx.config.poll_interval).await;
            }
        }
    }
    if let Some(until) = pause_until {
        let _ = ctx.pause.send(until);
    }
}

/// Picks the handler for `job.kind`.
async fn dispatch(ctx: &Context, job: &Job) -> JobOutcome {
    match job.kind.as_str() {
        TEST_PROMPT => test_prompt(ctx, &job.payload).await,
        CAPTURE_EXTRACT => capture_extract(ctx, &job.payload).await,
        AGENT_RUN => runs::execute(&ctx.pool, &ctx.runner, &ctx.runtime, &ctx.bus, job)
            .await
            .into(),
        other => JobOutcome::Fail(format!("unknown job kind {other:?}")),
    }
}

#[derive(Deserialize)]
struct TestPrompt {
    prompt: String,
}

/// `test_prompt`: one model call; the result mirrors `/dev/claude/ping`.
async fn test_prompt(ctx: &Context, payload: &Value) -> JobOutcome {
    let payload = match TestPrompt::deserialize(payload) {
        Ok(payload) => payload,
        Err(err) => return JobOutcome::Fail(format!("invalid {TEST_PROMPT} payload: {err}")),
    };
    let req = ClaudeRequest {
        prompt: payload.prompt,
        purpose: "job.test_prompt".into(),
        model: None,
    };
    match ctx.runner.call(&ctx.pool, &req).await {
        Ok(res) => JobOutcome::Done(json!({
            "text": res.text,
            "model": res.model,
            "duration_ms": res.duration_ms,
            "cost_usd": res.cost_usd,
            "log_id": res.log_id,
        })),
        Err(err) => claude_failure(&err),
    }
}

/// `capture_extract`: Claude reads a page's text; the result is a `Capture`.
async fn capture_extract(ctx: &Context, payload: &Value) -> JobOutcome {
    match capture::llm::extract(&ctx.runner, &ctx.pool, payload).await {
        Ok(capture) => match serde_json::to_value(capture) {
            Ok(result) => JobOutcome::Done(result),
            Err(err) => JobOutcome::Fail(format!("cannot serialize the capture: {err}")),
        },
        Err(LlmError::Payload(err)) => {
            JobOutcome::Fail(format!("invalid {CAPTURE_EXTRACT} payload: {err}"))
        }
        Err(LlmError::Claude(err)) => claude_failure(&err),
    }
}

impl From<runs::Outcome> for JobOutcome {
    fn from(outcome: runs::Outcome) -> Self {
        match outcome {
            runs::Outcome::Done(result) => Self::Done(result),
            runs::Outcome::Retry(error) => Self::Retry(error),
            runs::Outcome::UsageLimit(error) => Self::UsageLimit(error),
            runs::Outcome::Fail(error) => Self::Fail(error),
            runs::Outcome::Paused => Self::Paused,
        }
    }
}

/// A usage limit defers the job; every other runner error is worth a retry. The mapping
/// lives in `runs::StepError`, so jobs and run steps treat runner errors alike.
fn claude_failure(err: &ClaudeError) -> JobOutcome {
    runs::Outcome::from(runs::StepError::from(err.clone())).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let base = Duration::from_secs(10);
        let cap = Duration::from_secs(600);
        let cases = [
            (1, base),
            (2, base * 2),
            (3, base * 4),
            (7, cap),
            (i32::MAX, cap),
            (0, base),
            (i32::MIN, base),
        ];
        for (attempt, expected) in cases {
            assert_eq!(backoff(attempt, base, cap), expected, "attempt {attempt}");
        }
        let huge = Duration::MAX;
        assert_eq!(backoff(40, huge, huge), huge, "saturates without panicking");
    }
}
