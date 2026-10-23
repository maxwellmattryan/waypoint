//! The candidate profile import: positions, skills and education read from a LinkedIn
//! data export ([`linkedin`]) or from a resume version's text ([`resume`]), stored as
//! `fact` memories with their source.
//!
//! Every imported memory gets a source key built from its identity rather than its
//! wording ([`ProfileEntry::key`]). A key that is already stored is skipped and counted as
//! "already known", so re-imports and overlapping sources add no duplicates and never
//! change an existing memory.

pub mod linkedin;
pub mod resume;

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};

use crate::memories::{
    self, MAX_CONTENT, MAX_TAG, MemoryInput, MemoryKind, MemorySource, MemoryStatus, MemoryTopic,
    NewMemory, normalise_tag,
};

/// One position, skill or education entry, from either source. Every value is trimmed,
/// and an empty one is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileEntry {
    Position {
        company: String,
        title: String,
        location: Option<String>,
        started: Option<String>,
        finished: Option<String>,
        description: Option<String>,
    },
    Skill {
        name: String,
    },
    Education {
        school: String,
        degree: Option<String>,
        started: Option<String>,
        finished: Option<String>,
        notes: Option<String>,
    },
}

/// A value trimmed, with a blank one as `None`.
fn value(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

impl ProfileEntry {
    /// A position, or `None` when the company or title is blank.
    pub fn position(
        company: Option<&str>,
        title: Option<&str>,
        location: Option<&str>,
        started: Option<&str>,
        finished: Option<&str>,
        description: Option<&str>,
    ) -> Option<Self> {
        Some(Self::Position {
            company: value(company)?,
            title: value(title)?,
            location: value(location),
            started: value(started),
            finished: value(finished),
            description: value(description),
        })
    }

    /// A skill, or `None` when the name is blank.
    pub fn skill(name: Option<&str>) -> Option<Self> {
        Some(Self::Skill { name: value(name)? })
    }

    /// An education entry, or `None` when the school is blank.
    pub fn education(
        school: Option<&str>,
        degree: Option<&str>,
        started: Option<&str>,
        finished: Option<&str>,
        notes: Option<&str>,
    ) -> Option<Self> {
        Some(Self::Education {
            school: value(school)?,
            degree: value(degree),
            started: value(started),
            finished: value(finished),
            notes: value(notes),
        })
    }

    pub fn topic(&self) -> MemoryTopic {
        match self {
            Self::Position { .. } => MemoryTopic::Position,
            Self::Skill { .. } => MemoryTopic::Skill,
            Self::Education { .. } => MemoryTopic::Education,
        }
    }

    /// The entry's identity: equal keys are the same fact, whichever source it came from.
    pub fn key(&self) -> String {
        let n = normalise_tag;
        match self {
            Self::Position { company, title, .. } => {
                format!("position:{}|{}", n(company), n(title))
            }
            Self::Skill { name } => format!("skill:{}", n(name)),
            Self::Education { school, degree, .. } => format!(
                "education:{}|{}",
                n(school),
                n(degree.as_deref().unwrap_or(""))
            ),
        }
    }

    /// The company or school as a tag; none for a skill. A tag too long to store is
    /// dropped rather than failing the entry.
    pub fn tags(&self) -> Vec<String> {
        let tag = match self {
            Self::Position { company, .. } => Some(company),
            Self::Skill { .. } => None,
            Self::Education { school, .. } => Some(school),
        };
        tag.filter(|tag| normalise_tag(tag).chars().count() <= MAX_TAG)
            .map(|tag| vec![tag.clone()])
            .unwrap_or_default()
    }

    /// The memory's text, cut to [`MAX_CONTENT`] characters.
    pub fn content(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        let body = match self {
            Self::Position {
                company,
                title,
                location,
                started,
                finished,
                description,
            } => {
                let mut first = format!("{title} at {company}");
                if let Some(location) = location {
                    first.push_str(&format!(" · {location}"));
                }
                lines.push(first);
                match (started, finished) {
                    (Some(started), finished) => lines.push(format!(
                        "{started} – {}",
                        finished.as_deref().unwrap_or("present")
                    )),
                    (None, Some(finished)) => lines.push(format!("Until {finished}")),
                    (None, None) => {}
                }
                description
            }
            Self::Skill { name } => {
                lines.push(name.clone());
                &None
            }
            Self::Education {
                school,
                degree,
                started,
                finished,
                notes,
            } => {
                lines.push(match degree {
                    Some(degree) => format!("{degree} at {school}"),
                    None => school.clone(),
                });
                match (started, finished) {
                    (Some(started), Some(finished)) => {
                        lines.push(format!("{started} – {finished}"))
                    }
                    (Some(one), None) | (None, Some(one)) => lines.push(one.clone()),
                    (None, None) => {}
                }
                notes
            }
        };
        if let Some(body) = body {
            lines.push(String::new());
            lines.push(body.clone());
        }
        cut(&lines.join("\n"))
    }
}

/// `text` when it fits in [`MAX_CONTENT`] characters, otherwise its first
/// `MAX_CONTENT - 1` characters and `…`.
fn cut(text: &str) -> String {
    if text.chars().count() <= MAX_CONTENT {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_CONTENT - 1).collect();
    out.push('…');
    out
}

/// What an import did: the body of `POST /profile/linkedin` and the `resume_profile` job's
/// `result`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProfileImportSummary {
    /// New memories stored.
    pub added: u32,
    /// Entries whose key was already stored, in this import or an earlier one.
    pub already_known: u32,
    /// Entries missing a required value.
    pub skipped: u32,
}

/// Stores `entries` as `fact` memories with `status` and `source`, in order, counting each
/// into `summary`. An entry whose key is already stored is left alone.
pub async fn store(
    tx: &mut Transaction<'_, Postgres>,
    entries: &[ProfileEntry],
    status: MemoryStatus,
    source: &MemorySource,
    summary: &mut ProfileImportSummary,
) -> sqlx::Result<()> {
    for entry in entries {
        let input = MemoryInput {
            kind: MemoryKind::Fact,
            topic: entry.topic(),
            content: entry.content(),
            tags: entry.tags(),
        };
        // Content is cut and long tags dropped above, so this can't fail; count it anyway.
        let Ok(fields) = memories::validate(&input) else {
            summary.skipped += 1;
            continue;
        };
        let memory = NewMemory {
            fields,
            status,
            source: source.clone(),
        };
        match memories::insert_keyed(&mut **tx, &memory, &entry.key()).await? {
            Some(_) => summary.added += 1,
            None => summary.already_known += 1,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_content_keys_and_tags() {
        let long = "x".repeat(6_000);
        let long_tag = "Company ".repeat(10);
        let long_content = format!("Dev at {}", long_tag.trim());
        let long_key = format!("position:{}|dev", normalise_tag(&long_tag));
        let cases: Vec<(ProfileEntry, &str, &str, Vec<&str>)> = vec![
            (
                ProfileEntry::position(
                    Some(" Acme  Corp "),
                    Some("Staff Engineer"),
                    Some("Berlin"),
                    Some("Jan 2020"),
                    None,
                    Some("Built things."),
                )
                .unwrap(),
                "Staff Engineer at Acme  Corp · Berlin\nJan 2020 – present\n\nBuilt things.",
                "position:acme corp|staff engineer",
                vec!["Acme  Corp"],
            ),
            (
                ProfileEntry::position(
                    Some("Acme"),
                    Some("Engineer"),
                    None,
                    Some("Jan 2018"),
                    Some("Dec 2019"),
                    None,
                )
                .unwrap(),
                "Engineer at Acme\nJan 2018 – Dec 2019",
                "position:acme|engineer",
                vec!["Acme"],
            ),
            (
                ProfileEntry::position(
                    Some("Acme"),
                    Some("Intern"),
                    None,
                    None,
                    Some("2017"),
                    None,
                )
                .unwrap(),
                "Intern at Acme\nUntil 2017",
                "position:acme|intern",
                vec!["Acme"],
            ),
            (
                ProfileEntry::position(Some(&long_tag), Some("Dev"), None, None, None, None)
                    .unwrap(),
                &long_content,
                &long_key,
                vec![],
            ),
            (
                ProfileEntry::skill(Some("  Rust ")).unwrap(),
                "Rust",
                "skill:rust",
                vec![],
            ),
            (
                ProfileEntry::education(
                    Some("MIT"),
                    Some("BSc Computer Science"),
                    Some("2010"),
                    Some("2014"),
                    Some("Honours"),
                )
                .unwrap(),
                "BSc Computer Science at MIT\n2010 – 2014\n\nHonours",
                "education:mit|bsc computer science",
                vec!["MIT"],
            ),
            (
                ProfileEntry::education(Some("MIT"), None, None, Some("2014"), None).unwrap(),
                "MIT\n2014",
                "education:mit|",
                vec!["MIT"],
            ),
            (
                ProfileEntry::education(Some("MIT"), Some(" "), Some("2010"), None, None).unwrap(),
                "MIT\n2010",
                "education:mit|",
                vec!["MIT"],
            ),
        ];
        for (entry, content, key, tags) in cases {
            assert_eq!(entry.content(), content, "{entry:?}");
            assert_eq!(entry.key(), key, "{entry:?}");
            assert_eq!(entry.tags(), tags, "{entry:?}");
        }

        let skill = ProfileEntry::skill(Some(&long)).unwrap();
        let content = skill.content();
        assert_eq!(content.chars().count(), MAX_CONTENT);
        assert!(content.ends_with('…'));
        assert_eq!(
            content.chars().filter(|c| *c == 'x').count(),
            MAX_CONTENT - 1
        );

        assert_eq!(
            ProfileEntry::position(Some("Acme"), Some(" "), None, None, None, None),
            None
        );
        assert_eq!(
            ProfileEntry::position(None, Some("Dev"), None, None, None, None),
            None
        );
        assert_eq!(ProfileEntry::skill(Some("")), None);
        assert_eq!(
            ProfileEntry::education(Some("  "), Some("BSc"), None, None, None),
            None
        );

        let topics: Vec<MemoryTopic> = [
            ProfileEntry::skill(Some("Go")).unwrap(),
            ProfileEntry::education(Some("MIT"), None, None, None, None).unwrap(),
        ]
        .iter()
        .map(ProfileEntry::topic)
        .collect();
        assert_eq!(topics, [MemoryTopic::Skill, MemoryTopic::Education]);
    }
}
