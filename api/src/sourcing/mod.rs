//! Board sourcing: target companies' public Greenhouse, Lever and Ashby boards, kept in
//! `job_postings` by one `source_board` job per (source, board slug). See
//! `docs/sourcing.md`.

pub mod boards;
pub mod location;
pub mod store;

use serde::{Deserialize, Serialize};

/// A public job board this module reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Greenhouse,
    Lever,
    Ashby,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
        }
    }

    /// The exact lowercase name, as `as_str` gives it.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "greenhouse" => Some(Self::Greenhouse),
            "lever" => Some(Self::Lever),
            "ashby" => Some(Self::Ashby),
            _ => None,
        }
    }
}
