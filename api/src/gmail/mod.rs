//! Gmail sync (see docs/gmail-sync.md): a read-only Gmail client and a paced,
//! single-flight sync on the job queue that stores messages with [`crate::email::ingest`].

pub mod client;
pub mod sync;

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How a sync run reads the mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// No stored `history_id`, or another account: lists the backfill window.
    Backfill,
    /// The stored history expired: lists the backfill window again, keeping stored rows.
    Resync,
    /// Reads `history.list` from the stored `history_id`.
    Incremental,
}

impl SyncMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backfill => "backfill",
            Self::Resync => "resync",
            Self::Incremental => "incremental",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "backfill" => Some(Self::Backfill),
            "resync" => Some(Self::Resync),
            "incremental" => Some(Self::Incremental),
            _ => None,
        }
    }
}

/// What a page or a run did with the messages it listed. `seen` is the sum of the rest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SyncCounts {
    pub seen: i32,
    /// Stored as new rows.
    pub inserted: i32,
    /// Already stored; refreshed in place.
    pub updated: i32,
    /// Over the size cap; not stored.
    pub oversized: i32,
    /// Didn't parse or came back with another id; not stored.
    pub failed: i32,
    /// Deleted between listing and fetching.
    pub vanished: i32,
}

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
