//! The in-process event bus and `GET /events`, which streams it to clients as
//! server-sent events. There is no replay: a client that (re)connects refetches state.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::response::stream::{Event as SseEvent, EventStream};
use rocket::{Shutdown, State};
use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError};

use crate::applications::Application;
use crate::host::LocalHost;
use crate::jobs::Job;
use crate::runs::Run;

/// Events a slow subscriber can fall behind by before it gets [`Event::AppLagged`].
pub const DEFAULT_CAPACITY: usize = 256;
/// How often `/events` sends a comment so proxies keep the stream open.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// One event on the bus. It serializes with its name in `"type"`.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "job.queued")]
    JobQueued { job: Job },
    #[serde(rename = "job.started")]
    JobStarted { job: Job },
    #[serde(rename = "job.retrying")]
    JobRetrying { job: Job },
    #[serde(rename = "job.deferred")]
    JobDeferred { job: Job },
    #[serde(rename = "job.succeeded")]
    JobSucceeded { job: Job },
    #[serde(rename = "job.failed")]
    JobFailed { job: Job },
    #[serde(rename = "queue.paused")]
    QueuePaused { until: DateTime<Utc> },
    #[serde(rename = "queue.resumed")]
    QueueResumed,
    #[serde(rename = "app.connected")]
    AppConnected,
    #[serde(rename = "app.lagged")]
    AppLagged { skipped: u64 },
    /// An application was created; `application` is the `POST /applications` body.
    #[serde(rename = "application.created")]
    ApplicationCreated { application: Application },
    /// An application's text or status changed; `application` is the route's body.
    #[serde(rename = "application.updated")]
    ApplicationUpdated { application: Application },
    #[serde(rename = "application.deleted")]
    ApplicationDeleted { id: i64 },
    /// An agent run changed; `run` is the `agent_runs` row as `GET /runs` returns it.
    #[serde(rename = "run.updated")]
    RunUpdated { run: Run },
}

impl Event {
    /// The event's type, used as the SSE `event:` field.
    pub fn name(&self) -> &'static str {
        match self {
            Self::JobQueued { .. } => "job.queued",
            Self::JobStarted { .. } => "job.started",
            Self::JobRetrying { .. } => "job.retrying",
            Self::JobDeferred { .. } => "job.deferred",
            Self::JobSucceeded { .. } => "job.succeeded",
            Self::JobFailed { .. } => "job.failed",
            Self::QueuePaused { .. } => "queue.paused",
            Self::QueueResumed => "queue.resumed",
            Self::AppConnected => "app.connected",
            Self::AppLagged { .. } => "app.lagged",
            Self::ApplicationCreated { .. } => "application.created",
            Self::ApplicationUpdated { .. } => "application.updated",
            Self::ApplicationDeleted { .. } => "application.deleted",
            Self::RunUpdated { .. } => "run.updated",
        }
    }

    /// The job a `job.*` event carries.
    pub fn job(&self) -> Option<&Job> {
        match self {
            Self::JobQueued { job }
            | Self::JobStarted { job }
            | Self::JobRetrying { job }
            | Self::JobDeferred { job }
            | Self::JobSucceeded { job }
            | Self::JobFailed { job } => Some(job),
            _ => None,
        }
    }
}

/// Fans every published event out to every current subscriber.
#[derive(Debug, Clone)]
pub struct EventBus(broadcast::Sender<Event>);

impl EventBus {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(broadcast::channel(capacity).0)
    }

    /// Sends `event` to every subscriber. Having none is fine.
    pub fn publish(&self, event: Event) {
        let _ = self.0.send(event);
    }

    pub fn subscribe(&self) -> Subscriber {
        Subscriber(self.0.subscribe())
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// One subscriber's view of the bus.
#[derive(Debug)]
pub struct Subscriber(broadcast::Receiver<Event>);

impl Subscriber {
    /// The next event. A subscriber that fell behind the buffer gets
    /// [`Event::AppLagged`] in place of the events it missed. `None` once the bus is gone.
    pub async fn next(&mut self) -> Option<Event> {
        match self.0.recv().await {
            Ok(event) => Some(event),
            Err(RecvError::Lagged(skipped)) => Some(Event::AppLagged { skipped }),
            Err(RecvError::Closed) => None,
        }
    }
}

fn sse(event: &Event) -> SseEvent {
    SseEvent::json(event).event(event.name())
}

/// `GET /events`: `app.connected`, then every event published while connected.
#[utoipa::path(
    get,
    path = "/events",
    tag = "events",
    responses(
        (
            status = 200,
            description = "Server-sent events: `app.connected`, then every event published while connected. Each event's SSE `event:` field is its `type`; there is no replay.",
            content_type = "text/event-stream",
            body = Event,
        ),
        (status = 403, description = "The `Host` header is not a local address"),
    )
)]
#[rocket::get("/events")]
pub fn events(_host: LocalHost, bus: &State<EventBus>, mut shutdown: Shutdown) -> EventStream![] {
    // Subscribe now, not when the stream is first polled, so nothing published after
    // the response starts is missed.
    let mut subscriber = bus.subscribe();
    EventStream! {
        yield sse(&Event::AppConnected);
        loop {
            let event = rocket::tokio::select! {
                event = subscriber.next() => match event {
                    Some(event) => event,
                    None => break,
                },
                _ = &mut shutdown => break,
            };
            yield sse(&event);
        }
    }
    .heartbeat(HEARTBEAT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lagged_subscriber_gets_app_lagged() {
        let bus = EventBus::with_capacity(1);
        let mut subscriber = bus.subscribe();
        bus.publish(Event::AppConnected);
        bus.publish(Event::AppConnected);
        bus.publish(Event::QueueResumed);

        assert_eq!(
            subscriber.next().await,
            Some(Event::AppLagged { skipped: 2 })
        );
        assert_eq!(subscriber.next().await, Some(Event::QueueResumed));

        bus.publish(Event::AppLagged { skipped: 7 });
        assert_eq!(
            subscriber.next().await,
            Some(Event::AppLagged { skipped: 7 })
        );
    }

    #[test]
    fn events_serialize_with_their_type() {
        let event = Event::AppLagged { skipped: 3 };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({"type": "app.lagged", "skipped": 3})
        );
        assert_eq!(
            serde_json::to_value(Event::QueueResumed).unwrap(),
            serde_json::json!({"type": "queue.resumed"})
        );

        let now = Utc::now();
        let run = Run {
            id: 7,
            workflow: "w".into(),
            status: crate::runs::RunStatus::Queued,
            input: serde_json::json!({}),
            output: None,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            job_id: Some(1),
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
        };
        let event = Event::RunUpdated { run: run.clone() };
        assert_eq!(event.name(), "run.updated");
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({"type": "run.updated", "run": run})
        );
    }
}
