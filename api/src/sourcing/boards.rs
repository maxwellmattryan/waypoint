//! Each board's list API and the mappers from its entries to [`Posting`]s.

use chrono::{DateTime, Utc};

use super::location::Location;

/// One board entry, mapped: a `job_postings` row's board-derived columns.
#[derive(Debug, Clone, PartialEq)]
pub struct Posting {
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub url: String,
    pub apply_url: Option<String>,
    pub department: Option<String>,
    pub employment_type: Option<String>,
    pub location_raw: Option<String>,
    pub location: Location,
    pub description: String,
    pub posted_at: Option<DateTime<Utc>>,
}
