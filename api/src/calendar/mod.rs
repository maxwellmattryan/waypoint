//! Google Calendar: one write, `events.insert` on the user's `primary` calendar, made only
//! by the `create_calendar_event` tool after the user approves it (see
//! [`crate::workflows::propose_interview`] and docs/google-oauth.md). The event has no
//! attendees and Google sends no updates (`sendUpdates=none`).

pub mod client;

pub use self::client::{CalendarClient, CalendarError, CreatedEvent, NewEvent};

/// Google Calendar's API origin.
pub const BASE_URL: &str = "https://www.googleapis.com";

/// Calendar settings: a code default that tests override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The API origin, e.g. `https://www.googleapis.com`.
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.into(),
        }
    }
}
