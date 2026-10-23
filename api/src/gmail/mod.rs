//! Gmail sync (see docs/gmail-sync.md): a read-only Gmail client and a paced,
//! single-flight sync on the job queue that stores messages with [`crate::email::ingest`].

pub mod client;

use std::time::Duration;

/// Gmail sync settings. Code defaults; tests override them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The Gmail API origin.
    pub base_url: String,
    /// How far back a backfill or resync reads (`newer_than:<days>d`).
    pub backfill_days: u32,
    /// Most messages a backfill or resync lists.
    pub max_messages: u32,
    /// Ids per page (`maxResults`).
    pub page_size: u32,
    /// Delay before the next page's job may start.
    pub page_delay: Duration,
    /// Largest decoded message stored; bigger ones count as `oversized`.
    pub max_message_bytes: usize,
    /// Startup catch-up runs when the last sync finished longer ago than this.
    pub stale_after: Duration,
}

pub const BASE_URL: &str = "https://gmail.googleapis.com";

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.into(),
            backfill_days: 30,
            max_messages: 1000,
            page_size: 100,
            page_delay: Duration::from_secs(2),
            max_message_bytes: 5 * 1024 * 1024,
            stale_after: Duration::from_secs(10 * 60),
        }
    }
}
