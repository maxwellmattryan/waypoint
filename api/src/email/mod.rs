//! Recruiter email, offline part (see docs/email.md): parsing raw RFC 822 messages,
//! normalizing their text, grouping them into threads and deciding with cheap sender
//! heuristics which ones are worth a model call. Nothing here calls a model or the network.

pub mod heuristics;
pub mod html;
pub mod mime;
pub mod normalize;
pub mod quote;
pub mod store;
pub mod thread;

use std::fmt;

pub use store::{EmailSummary, IngestError, Ingested, NewEmail, ingest, known_companies, list};

/// Where a message came from: `emails.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A synthetic file under `api/fixtures/emails/`.
    Fixture,
    /// The user's Gmail inbox.
    Gmail,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
            Self::Gmail => "gmail",
        }
    }
}

/// Whether a message is worth a model call: `emails.verdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictKind {
    Candidate,
    Skip,
}

impl VerdictKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Skip => "skip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "candidate" => Some(Self::Candidate),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

impl fmt::Display for VerdictKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An applicant tracking system recognised by sender or link: `emails.ats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ats {
    Greenhouse,
    Lever,
    Ashby,
    Workday,
}

impl Ats {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
            Self::Workday => "workday",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "greenhouse" => Some(Self::Greenhouse),
            "lever" => Some(Self::Lever),
            "ashby" => Some(Self::Ashby),
            "workday" => Some(Self::Workday),
            _ => None,
        }
    }
}

impl fmt::Display for Ats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which part `emails.body_text` came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodySource {
    /// A non-blank text/plain part.
    Plain,
    /// The text/html part, converted to text.
    Html,
    /// The message has no text or HTML part.
    None,
}

impl BodySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Html => "html",
            Self::None => "none",
        }
    }
}
