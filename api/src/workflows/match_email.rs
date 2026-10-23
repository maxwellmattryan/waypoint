//! `match_email`: matches one classified email to an application and acts on it (see
//! docs/email.md, "Match and act"). Matching is deterministic Rust, with no model call.
//!
//! **Keys.** A company key is [`heuristics::company_key`] of the name after trailing
//! legal-suffix words (`inc`, `corp`, `llc`, …) are dropped, and counts only with at least
//! 3 characters. Role tokens are the lowercase alphanumeric words of a role or title,
//! without `the a an of for and at to role position`.
//!
//! **Signals.** Each application scores the points of the signals that fire, clamped to
//! 0..=1 ([`score`]):
//!
//! | Reason | Fires when | Points |
//! |---|---|---|
//! | `thread` | another email of the thread is linked (`applied`/`no_action`) to it | +0.6 |
//! | `company` | the classification's company key equals its company key | +0.5, shared |
//! | `sender_name` | the From name's key equals or starts with its company key | shared |
//! | `sender_domain` | a non-ATS, non-free-mail From domain has a label equal to its company key, or was linked to it before | +0.3 |
//! | `role` | role and title tokens are equal, non-empty sets | +0.3 |
//! | `role_partial` | their Jaccard overlap is ≥ 0.5 and < 1 | +0.15 |
//! | `role_mismatch` | both are non-empty and share no token | −0.3 |
//!
//! `company` and `sender_name` together count once. Points are kept in hundredths, so
//! sums and thresholds are exact.
//!
//! **Choosing** ([`choose`]). A candidate scores ≥ 0.5; the top 3 are kept, by score
//! descending then id. The match is ambiguous when the second candidate is within 0.1 of
//! the top one, and linked when the top one scores ≥ [`REVIEW_THRESHOLD`] and isn't
//! ambiguous.
//!
//! **Planning** ([`plan`]) reads the top candidate's status and stages: a rejection moves
//! it to `rejected`, an offer to `offer`, an invite or availability request to
//! `interviewing` (from `rejected` too, marked `unexpected_status`) and adds the stated
//! stage when it is new and the application has fewer than 50 stages.
//!
//! **Routing** ([`route`]). A linked, confident (≥ 0.8) match without an unexpected status
//! applies its ops through the `write` tools [`SetApplicationStatus`] and
//! [`AddApplicationStage`] (each undoable); a linked match without ops is `no_action`; any
//! other candidate with ops goes to the review queue as `email.match`; no candidate but a
//! company proposes an `email.new_application`; everything else is `unmatched`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::applications::ApplicationStatus;
use crate::applications::stages::MAX_STAGES;
use crate::email::heuristics;
use crate::runs::REVIEW_THRESHOLD;
use crate::workflows::classify_email::EmailClass;

/// The workflow's name.
pub const WORKFLOW: &str = "match_email";
/// Its one step.
pub const STEP: &str = "match";
/// The review item kind of an unsure match to an existing application.
pub const MATCH_REVIEW_KIND: &str = "email.match";
/// The review item kind of a proposed new application.
pub const NEW_APPLICATION_REVIEW_KIND: &str = "email.new_application";
/// Most candidates kept.
pub const MAX_CANDIDATES: usize = 3;
/// The title of a proposed application whose email states no role.
pub const UNKNOWN_ROLE: &str = "Unknown role";

/// Words dropped from the end of a company name before its key is taken.
const LEGAL_SUFFIXES: [&str; 13] = [
    "inc",
    "incorporated",
    "corp",
    "corporation",
    "co",
    "company",
    "llc",
    "ltd",
    "limited",
    "plc",
    "gmbh",
    "ag",
    "sa",
];
/// Words role tokens leave out.
const ROLE_STOPWORDS: [&str; 10] = [
    "the", "a", "an", "of", "for", "and", "at", "to", "role", "position",
];
/// Sender domains of personal mailboxes, which say nothing about a company.
const FREE_MAIL: [&str; 11] = [
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "yahoo.com",
    "icloud.com",
    "me.com",
    "proton.me",
    "protonmail.com",
    "aol.com",
];
/// Shortest company key that counts.
const MIN_KEY: usize = 3;

// Points, in hundredths.
const THREAD_POINTS: i32 = 60;
const COMPANY_POINTS: i32 = 50;
const DOMAIN_POINTS: i32 = 30;
const ROLE_POINTS: i32 = 30;
const ROLE_PARTIAL_POINTS: i32 = 15;
const ROLE_MISMATCH_POINTS: i32 = -30;
/// A candidate scores at least this.
const CANDIDATE_POINTS: i32 = 50;
/// The second candidate within this of the top one makes the match ambiguous.
const AMBIGUOUS_POINTS: i32 = 10;

/// Hundredths of [`REVIEW_THRESHOLD`].
fn linked_points() -> i32 {
    (REVIEW_THRESHOLD * 100.0).round() as i32
}

/// The company key of `name` (see the module docs), or `None` when it is under 3
/// characters.
pub fn company_key(name: &str) -> Option<String> {
    let mut words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    while words
        .last()
        .is_some_and(|word| LEGAL_SUFFIXES.contains(&word.to_lowercase().as_str()))
    {
        words.pop();
    }
    let key = heuristics::company_key(&words.concat());
    (key.chars().count() >= MIN_KEY).then_some(key)
}

/// The role tokens of `text` (see the module docs).
pub fn role_tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .filter(|word| !ROLE_STOPWORDS.contains(&word.as_str()))
        .collect()
}

/// What the matcher reads from the email and its classification.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmailFacts<'a> {
    /// The classification's `company`.
    pub company: Option<&'a str>,
    /// The classification's `role`.
    pub role: Option<&'a str>,
    pub from_name: Option<&'a str>,
    pub from_address: Option<&'a str>,
}

/// What the matcher reads from one application.
#[derive(Debug, Clone, PartialEq)]
pub struct AppFacts {
    pub id: i64,
    pub company: String,
    pub title: String,
    pub status: ApplicationStatus,
}

/// What earlier links say about one application.
#[derive(Debug, Clone, Default)]
pub struct History {
    /// Another email of this email's thread is linked to the application.
    pub thread: bool,
    /// The From domains (lowercase) of other emails linked to the application.
    pub domains: BTreeSet<String>,
}

/// One application's score: hundredths, clamped to 0..=100, and the reasons that fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scored {
    pub points: i32,
    pub reasons: Vec<&'static str>,
}

impl Scored {
    pub fn score(&self) -> f64 {
        f64::from(self.points) / 100.0
    }
}

/// The From domain, lowercase, unless it is an ATS sender or a free-mail domain.
fn company_domain(from_address: Option<&str>) -> Option<String> {
    let domain = heuristics::domain(from_address?)?.trim().to_lowercase();
    let excluded = heuristics::ATS_SENDERS
        .iter()
        .map(|(sender, _)| *sender)
        .chain(FREE_MAIL)
        .any(|sender| heuristics::domain_matches(&domain, sender));
    (!excluded && !domain.is_empty()).then_some(domain)
}

/// Scores one application for an email (see the module docs).
pub fn score(email: &EmailFacts<'_>, app: &AppFacts, history: &History) -> Scored {
    let mut points = 0;
    let mut reasons = Vec::new();
    let app_key = company_key(&app.company);

    if history.thread {
        points += THREAD_POINTS;
        reasons.push("thread");
    }

    let company = app_key.is_some() && email.company.and_then(company_key) == app_key;
    let sender_name = app_key.as_deref().is_some_and(|key| {
        email
            .from_name
            .map(heuristics::company_key)
            .is_some_and(|name| name.starts_with(key))
    });
    if company {
        reasons.push("company");
    }
    if sender_name {
        reasons.push("sender_name");
    }
    if company || sender_name {
        points += COMPANY_POINTS;
    }

    if let Some(domain) = company_domain(email.from_address) {
        let labels: Vec<&str> = domain.split('.').collect();
        let by_label = app_key.as_deref().is_some_and(|key| {
            labels[..labels.len().saturating_sub(1)]
                .iter()
                .any(|label| heuristics::company_key(label) == key)
        });
        if by_label || history.domains.contains(&domain) {
            points += DOMAIN_POINTS;
            reasons.push("sender_domain");
        }
    }

    let role = email.role.map(role_tokens).unwrap_or_default();
    let title = role_tokens(&app.title);
    if !role.is_empty() && !title.is_empty() {
        let shared = role.intersection(&title).count();
        let union = role.union(&title).count();
        if shared == union {
            points += ROLE_POINTS;
            reasons.push("role");
        } else if shared == 0 {
            points += ROLE_MISMATCH_POINTS;
            reasons.push("role_mismatch");
        } else if shared * 2 >= union {
            points += ROLE_PARTIAL_POINTS;
            reasons.push("role_partial");
        }
    }

    Scored {
        points: points.clamp(0, 100),
        reasons,
    }
}

/// A candidate, as review payloads and the match row list it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub application_id: i64,
    pub company: String,
    pub title: String,
    pub status: ApplicationStatus,
    pub score: f64,
    pub reasons: Vec<String>,
    /// The score in hundredths, for exact comparisons.
    #[serde(skip)]
    pub points: i32,
}

/// What [`choose`] decided.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Choice {
    /// Up to [`MAX_CANDIDATES`], best first.
    pub candidates: Vec<Candidate>,
    pub ambiguous: bool,
    pub linked: bool,
}

impl Choice {
    pub fn top(&self) -> Option<&Candidate> {
        self.candidates.first()
    }
}

/// Keeps the candidates among `scored` and decides whether the top one is linked (see the
/// module docs).
pub fn choose(scored: Vec<(AppFacts, Scored)>) -> Choice {
    let mut candidates: Vec<Candidate> = scored
        .into_iter()
        .filter(|(_, scored)| scored.points >= CANDIDATE_POINTS)
        .map(|(app, scored)| Candidate {
            application_id: app.id,
            company: app.company,
            title: app.title,
            status: app.status,
            score: scored.score(),
            reasons: scored.reasons.iter().map(|r| (*r).to_owned()).collect(),
            points: scored.points,
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.points
            .cmp(&a.points)
            .then(a.application_id.cmp(&b.application_id))
    });
    candidates.truncate(MAX_CANDIDATES);
    let ambiguous = match candidates.as_slice() {
        [top, second, ..] => second.points >= top.points - AMBIGUOUS_POINTS,
        _ => false,
    };
    let linked = !ambiguous
        && candidates
            .first()
            .is_some_and(|top| top.points >= linked_points());
    Choice {
        candidates,
        ambiguous,
        linked,
    }
}

/// One planned change to an application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    SetStatus {
        from: ApplicationStatus,
        to: ApplicationStatus,
    },
    AddStage {
        name: String,
    },
}

/// The changes an email asks of the top candidate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub ops: Vec<Op>,
    /// A status change from a status the email shouldn't follow (an offer or invite for a
    /// rejected application); it always goes to review.
    pub unexpected_status: bool,
}

/// The ops for an email of `class` (stating `stage`) to an application with `status` and
/// `stages` (their names). See the module docs.
pub fn plan(
    class: EmailClass,
    status: ApplicationStatus,
    stages: &[String],
    stage: Option<&str>,
) -> Plan {
    use ApplicationStatus::{Applied, Interviewing, Offer, Rejected, Sourced};
    let mut result = Plan::default();
    let mut move_to = |to: ApplicationStatus, unexpected: bool| {
        result.ops.push(Op::SetStatus { from: status, to });
        result.unexpected_status |= unexpected;
    };
    match class {
        EmailClass::Rejection => {
            if status != Rejected {
                move_to(Rejected, false);
            }
        }
        EmailClass::Offer => match status {
            Sourced | Applied | Interviewing => move_to(Offer, false),
            Rejected => move_to(Offer, true),
            Offer => {}
        },
        EmailClass::InterviewInvite | EmailClass::AvailabilityRequest => {
            match status {
                Sourced | Applied => move_to(Interviewing, false),
                Rejected => move_to(Interviewing, true),
                Interviewing | Offer => {}
            }
            let name = stage.map(str::trim).filter(|name| !name.is_empty());
            if let Some(name) = name {
                let wanted = name.to_lowercase();
                let exists = stages
                    .iter()
                    .any(|existing| existing.trim().to_lowercase() == wanted);
                let room = i64::try_from(stages.len()).unwrap_or(i64::MAX) < MAX_STAGES;
                if !exists && room {
                    result.ops.push(Op::AddStage {
                        name: name.to_owned(),
                    });
                }
            }
        }
        EmailClass::Other => {}
    }
    result
}

/// A proposed new application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub company: String,
    pub title: String,
    pub status: ApplicationStatus,
}

/// The application an unmatched email with a company proposes: the role as its title
/// (`Unknown role` without one) and the status its class implies (`other` is `applied`
/// for an ATS email, else `sourced`).
pub fn proposal(class: EmailClass, company: &str, role: Option<&str>, ats: bool) -> Proposal {
    let status = match class {
        EmailClass::Rejection => ApplicationStatus::Rejected,
        EmailClass::Offer => ApplicationStatus::Offer,
        EmailClass::InterviewInvite | EmailClass::AvailabilityRequest => {
            ApplicationStatus::Interviewing
        }
        EmailClass::Other if ats => ApplicationStatus::Applied,
        EmailClass::Other => ApplicationStatus::Sourced,
    };
    Proposal {
        company: company.trim().to_owned(),
        title: role
            .map(str::trim)
            .filter(|role| !role.is_empty())
            .unwrap_or(UNKNOWN_ROLE)
            .to_owned(),
        status,
    }
}

/// Where a match goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Apply the ops through the `write` tools.
    Apply,
    /// Linked, nothing to change.
    NoAction,
    /// An `email.match` review item, with its `review_reasons`.
    Review(Vec<&'static str>),
    /// An `email.new_application` review item.
    NewApplication,
    Unmatched,
}

/// Routes a match: `plan` is the top candidate's, `confidence` the classification's and
/// `has_company` whether it names a company. See the module docs.
pub fn route(choice: &Choice, plan: &Plan, confidence: f64, has_company: bool) -> Route {
    let Some(top) = choice.top() else {
        return if has_company {
            Route::NewApplication
        } else {
            Route::Unmatched
        };
    };
    let confident = confidence.is_finite() && confidence >= REVIEW_THRESHOLD;
    if choice.linked && plan.ops.is_empty() {
        return Route::NoAction;
    }
    if plan.ops.is_empty() {
        return Route::Unmatched;
    }
    if choice.linked && confident && !plan.unexpected_status {
        return Route::Apply;
    }
    let mut reasons = Vec::new();
    if top.points < linked_points() || !confident {
        reasons.push("low_confidence");
    }
    if choice.ambiguous {
        reasons.push("ambiguous");
    }
    if plan.unexpected_status {
        reasons.push("unexpected_status");
    }
    Route::Review(reasons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ApplicationStatus::{Applied, Interviewing, Offer, Rejected, Sourced};

    fn app(id: i64, company: &str, title: &str) -> AppFacts {
        AppFacts {
            id,
            company: company.into(),
            title: title.into(),
            status: Applied,
        }
    }

    fn email<'a>(
        company: Option<&'a str>,
        role: Option<&'a str>,
        from_name: Option<&'a str>,
        from_address: Option<&'a str>,
    ) -> EmailFacts<'a> {
        EmailFacts {
            company,
            role,
            from_name,
            from_address,
        }
    }

    fn history(thread: bool, domains: &[&str]) -> History {
        History {
            thread,
            domains: domains.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    #[test]
    fn company_keys_and_role_tokens() {
        let keys = [
            ("Acme Corp", Some("acme")),
            ("Acme, Inc.", Some("acme")),
            ("Acme Co Ltd", Some("acme")),
            ("Vandelay Industries", Some("vandelayindustries")),
            ("Initech LLC", Some("initech")),
            ("SA Holdings", Some("saholdings")),
            ("AB", None),
            ("AB Corp", None),
            ("Corp", None),
            ("  ", None),
        ];
        for (name, want) in keys {
            assert_eq!(company_key(name).as_deref(), want, "{name}");
        }
        let tokens: Vec<String> = role_tokens("The Senior Engineer, Platform (role)")
            .into_iter()
            .collect();
        assert_eq!(tokens, ["engineer", "platform", "senior"]);
        assert!(role_tokens("the role").is_empty());
    }

    #[test]
    fn score_cases() {
        let vandelay = app(1, "Vandelay Industries", "Senior Engineer");
        let none = History::default();
        // (case, email, application, history, points, reasons)
        type Case<'a> = (
            &'a str,
            EmailFacts<'a>,
            &'a AppFacts,
            History,
            i32,
            &'a [&'a str],
        );
        let acme = app(2, "Acme Corp", "Backend Engineer");
        let soylent = app(3, "Soylent", "Staff Engineer");
        let hooli = app(4, "Hooli", "Data Engineer, Platform");
        let short = app(5, "AB", "Engineer");
        let cases: Vec<Case> = vec![
            (
                "company and role",
                email(
                    Some("Vandelay Industries"),
                    Some("Senior Engineer"),
                    None,
                    None,
                ),
                &vandelay,
                none.clone(),
                80,
                &["company", "role"],
            ),
            (
                "legal suffixes are dropped on both sides",
                email(Some("ACME, Inc."), None, None, None),
                &acme,
                none.clone(),
                50,
                &["company"],
            ),
            (
                "sender name equal to the company key",
                email(None, None, Some("Vandelay Industries"), None),
                &vandelay,
                none.clone(),
                50,
                &["sender_name"],
            ),
            (
                "sender name starting with the company key",
                email(None, None, Some("Hooli Talent"), None),
                &hooli,
                none.clone(),
                50,
                &["sender_name"],
            ),
            (
                "company and sender name count once",
                email(Some("Hooli"), None, Some("Hooli Talent"), None),
                &hooli,
                none.clone(),
                50,
                &["company", "sender_name"],
            ),
            (
                "near miss: sender name only containing the key",
                email(None, None, Some("Team Hooli"), None),
                &hooli,
                none.clone(),
                0,
                &[],
            ),
            (
                "sender domain label equal to the company key",
                email(None, None, None, Some("dana@soylent.example")),
                &soylent,
                none.clone(),
                30,
                &["sender_domain"],
            ),
            (
                "sender domain on a subdomain label",
                email(None, None, None, Some("ana@careers.acme.example")),
                &acme,
                none.clone(),
                30,
                &["sender_domain"],
            ),
            (
                "near miss: the TLD is not a label",
                email(None, None, None, Some("x@mail.soylent")),
                &soylent,
                none.clone(),
                0,
                &[],
            ),
            (
                "an ATS sender domain never counts",
                email(None, None, None, Some("no-reply@hooli.lever.co")),
                &hooli,
                history(false, &["hooli.lever.co"]),
                0,
                &[],
            ),
            (
                "a free-mail domain never counts",
                email(None, None, None, Some("dana@gmail.com")),
                &soylent,
                history(false, &["gmail.com"]),
                0,
                &[],
            ),
            (
                "sender domain from history",
                email(None, None, None, Some("Dana@People.Example")),
                &soylent,
                history(false, &["people.example"]),
                30,
                &["sender_domain"],
            ),
            (
                "thread history",
                email(None, None, None, None),
                &soylent,
                history(true, &[]),
                60,
                &["thread"],
            ),
            (
                "every signal clamps to 1",
                email(
                    Some("Soylent"),
                    Some("Staff Engineer"),
                    Some("Soylent"),
                    Some("dana@soylent.example"),
                ),
                &soylent,
                history(true, &[]),
                100,
                &["thread", "company", "sender_name", "sender_domain", "role"],
            ),
            (
                "role partial (2 of 3 tokens)",
                email(Some("Hooli"), Some("Data Engineer"), None, None),
                &hooli,
                none.clone(),
                65,
                &["company", "role_partial"],
            ),
            (
                "near miss: overlap under half is no signal",
                email(Some("Hooli"), Some("Senior Data Scientist"), None, None),
                &hooli,
                none.clone(),
                50,
                &["company"],
            ),
            (
                "role mismatch",
                email(Some("Vandelay Industries"), Some("Designer"), None, None),
                &vandelay,
                none.clone(),
                20,
                &["company", "role_mismatch"],
            ),
            (
                "a mismatch alone clamps to 0",
                email(None, Some("Designer"), None, None),
                &vandelay,
                none.clone(),
                0,
                &["role_mismatch"],
            ),
            (
                "role only stopwords is no signal",
                email(None, Some("The role"), None, None),
                &vandelay,
                none.clone(),
                0,
                &[],
            ),
            (
                "a short company key never matches",
                email(Some("AB"), None, Some("AB"), Some("x@ab.example")),
                &short,
                none.clone(),
                0,
                &[],
            ),
        ];
        for (name, facts, application, history, points, reasons) in cases {
            let got = score(&facts, application, &history);
            assert_eq!(got.points, points, "{name}: {:?}", got.reasons);
            assert_eq!(got.reasons, reasons, "{name}");
        }
    }

    fn scored(id: i64, points: i32) -> (AppFacts, Scored) {
        (
            app(id, &format!("Company {id}"), "Engineer"),
            Scored {
                points,
                reasons: vec!["company"],
            },
        )
    }

    #[test]
    fn choose_cases() {
        // (case, scored, candidate ids, ambiguous, linked)
        let cases: Vec<(&str, Vec<(AppFacts, Scored)>, Vec<i64>, bool, bool)> = vec![
            ("nothing scored", vec![], vec![], false, false),
            (
                "under 0.5 is no candidate",
                vec![scored(1, 45)],
                vec![],
                false,
                false,
            ),
            (
                "one candidate at 0.8 is linked",
                vec![scored(1, 80), scored(2, 30)],
                vec![1],
                false,
                true,
            ),
            (
                "one candidate at 0.75 is not",
                vec![scored(1, 75)],
                vec![1],
                false,
                false,
            ),
            (
                "a second within 0.1 is ambiguous",
                vec![scored(1, 80), scored(2, 90)],
                vec![2, 1],
                true,
                false,
            ),
            (
                "a second further than 0.1 is not",
                vec![scored(1, 80), scored(2, 95)],
                vec![2, 1],
                false,
                true,
            ),
            (
                "ties go to the lower id, top 3 kept",
                vec![scored(4, 50), scored(3, 50), scored(2, 50), scored(1, 50)],
                vec![1, 2, 3],
                true,
                false,
            ),
        ];
        for (name, input, ids, ambiguous, linked) in cases {
            let choice = choose(input);
            let got: Vec<i64> = choice
                .candidates
                .iter()
                .map(|candidate| candidate.application_id)
                .collect();
            assert_eq!(got, ids, "{name}");
            assert_eq!(choice.ambiguous, ambiguous, "{name}");
            assert_eq!(choice.linked, linked, "{name}");
        }
        let choice = choose(vec![scored(7, 80)]);
        assert_eq!(choice.candidates[0].score, 0.8);
        assert_eq!(
            serde_json::to_value(&choice.candidates[0]).unwrap(),
            serde_json::json!({"application_id": 7, "company": "Company 7",
                               "title": "Engineer", "status": "applied", "score": 0.8,
                               "reasons": ["company"]})
        );
    }

    fn status_op(from: ApplicationStatus, to: ApplicationStatus) -> Op {
        Op::SetStatus { from, to }
    }

    fn stage_op(name: &str) -> Op {
        Op::AddStage { name: name.into() }
    }

    #[test]
    fn plan_cases() {
        use EmailClass::{AvailabilityRequest, InterviewInvite, Other};
        let rejection = EmailClass::Rejection;
        let offer = EmailClass::Offer;
        let none: &[String] = &[];
        // (class, status, ops, unexpected) without a stage.
        let table: Vec<(EmailClass, ApplicationStatus, Vec<Op>, bool)> = vec![
            (
                rejection,
                Sourced,
                vec![status_op(Sourced, Rejected)],
                false,
            ),
            (
                rejection,
                Applied,
                vec![status_op(Applied, Rejected)],
                false,
            ),
            (
                rejection,
                Interviewing,
                vec![status_op(Interviewing, Rejected)],
                false,
            ),
            (rejection, Offer, vec![status_op(Offer, Rejected)], false),
            (rejection, Rejected, vec![], false),
            (offer, Sourced, vec![status_op(Sourced, Offer)], false),
            (offer, Applied, vec![status_op(Applied, Offer)], false),
            (
                offer,
                Interviewing,
                vec![status_op(Interviewing, Offer)],
                false,
            ),
            (offer, Offer, vec![], false),
            (offer, Rejected, vec![status_op(Rejected, Offer)], true),
            (
                InterviewInvite,
                Sourced,
                vec![status_op(Sourced, Interviewing)],
                false,
            ),
            (
                InterviewInvite,
                Applied,
                vec![status_op(Applied, Interviewing)],
                false,
            ),
            (InterviewInvite, Interviewing, vec![], false),
            (InterviewInvite, Offer, vec![], false),
            (
                InterviewInvite,
                Rejected,
                vec![status_op(Rejected, Interviewing)],
                true,
            ),
            (
                AvailabilityRequest,
                Applied,
                vec![status_op(Applied, Interviewing)],
                false,
            ),
            (AvailabilityRequest, Interviewing, vec![], false),
            (
                AvailabilityRequest,
                Rejected,
                vec![status_op(Rejected, Interviewing)],
                true,
            ),
        ];
        for (class, status, ops, unexpected) in table {
            let got = plan(class, status, none, None);
            assert_eq!(got.ops, ops, "{class} from {status:?}");
            assert_eq!(got.unexpected_status, unexpected, "{class} from {status:?}");
        }
        for status in ApplicationStatus::ALL {
            assert_eq!(plan(Other, status, none, Some("Onsite")), Plan::default());
        }

        // Stages.
        let onsite = vec!["Recruiter call".to_owned(), " ONSITE ".to_owned()];
        let full: Vec<String> = (0..50).map(|i| format!("Stage {i}")).collect();
        let almost: Vec<String> = (0..49).map(|i| format!("Stage {i}")).collect();
        let stage_cases: Vec<(
            &str,
            EmailClass,
            ApplicationStatus,
            &[String],
            Option<&str>,
            Vec<Op>,
        )> = vec![
            (
                "a new stage with the status change",
                InterviewInvite,
                Applied,
                &onsite[..1],
                Some(" Onsite "),
                vec![status_op(Applied, Interviewing), stage_op("Onsite")],
            ),
            (
                "an existing stage, any case and spacing",
                InterviewInvite,
                Interviewing,
                &onsite,
                Some("onsite"),
                vec![],
            ),
            (
                "a new stage while interviewing",
                AvailabilityRequest,
                Interviewing,
                &onsite,
                Some("Technical interview"),
                vec![stage_op("Technical interview")],
            ),
            (
                "a new stage at offer",
                InterviewInvite,
                Offer,
                none,
                Some("Team match"),
                vec![stage_op("Team match")],
            ),
            (
                "a blank stage",
                InterviewInvite,
                Interviewing,
                none,
                Some("  "),
                vec![],
            ),
            (
                "50 stages already",
                InterviewInvite,
                Interviewing,
                &full,
                Some("Onsite"),
                vec![],
            ),
            (
                "49 stages leave room",
                InterviewInvite,
                Interviewing,
                &almost,
                Some("Onsite"),
                vec![stage_op("Onsite")],
            ),
            (
                "a rejection ignores the stage",
                rejection,
                Applied,
                none,
                Some("Onsite"),
                vec![status_op(Applied, Rejected)],
            ),
            (
                "an offer ignores the stage",
                offer,
                Interviewing,
                none,
                Some("Onsite"),
                vec![status_op(Interviewing, Offer)],
            ),
        ];
        for (name, class, status, stages, stage, ops) in stage_cases {
            assert_eq!(plan(class, status, stages, stage).ops, ops, "{name}");
        }

        assert_eq!(
            serde_json::to_value([status_op(Applied, Rejected), stage_op("Onsite")]).unwrap(),
            serde_json::json!([{"op": "set_status", "from": "applied", "to": "rejected"},
                               {"op": "add_stage", "name": "Onsite"}])
        );
    }

    #[test]
    fn proposal_cases() {
        use EmailClass::{AvailabilityRequest, InterviewInvite, Other};
        let cases = [
            (
                EmailClass::Rejection,
                Some("Data Engineer"),
                false,
                "Data Engineer",
                Rejected,
            ),
            (EmailClass::Offer, None, false, UNKNOWN_ROLE, Offer),
            (InterviewInvite, Some(" SRE "), false, "SRE", Interviewing),
            (
                AvailabilityRequest,
                Some(""),
                true,
                UNKNOWN_ROLE,
                Interviewing,
            ),
            (Other, Some("Data Engineer"), true, "Data Engineer", Applied),
            (
                Other,
                Some("Data Engineer"),
                false,
                "Data Engineer",
                Sourced,
            ),
        ];
        for (class, role, ats, title, status) in cases {
            assert_eq!(
                proposal(class, " Globex ", role, ats),
                Proposal {
                    company: "Globex".into(),
                    title: title.into(),
                    status,
                },
                "{class} {role:?} {ats}"
            );
        }
    }

    #[test]
    fn route_cases() {
        let candidate = |points: i32| Candidate {
            application_id: 1,
            company: "Acme".into(),
            title: "Engineer".into(),
            status: Applied,
            score: f64::from(points) / 100.0,
            reasons: Vec::new(),
            points,
        };
        let choice = |points: Option<i32>, ambiguous: bool, linked: bool| Choice {
            candidates: points.into_iter().map(candidate).collect(),
            ambiguous,
            linked,
        };
        let ops = Plan {
            ops: vec![status_op(Applied, Rejected)],
            unexpected_status: false,
        };
        let unexpected = Plan {
            ops: vec![status_op(Rejected, Offer)],
            unexpected_status: true,
        };
        let empty = Plan::default();
        let cases: Vec<(&str, Choice, &Plan, f64, bool, Route)> = vec![
            (
                "linked and confident",
                choice(Some(80), false, true),
                &ops,
                0.8,
                false,
                Route::Apply,
            ),
            (
                "linked without ops",
                choice(Some(90), false, true),
                &empty,
                0.3,
                false,
                Route::NoAction,
            ),
            (
                "linked, unsure classification",
                choice(Some(90), false, true),
                &ops,
                0.79,
                false,
                Route::Review(vec!["low_confidence"]),
            ),
            (
                "a NaN confidence is unsure",
                choice(Some(90), false, true),
                &ops,
                f64::NAN,
                false,
                Route::Review(vec!["low_confidence"]),
            ),
            (
                "a weak candidate",
                choice(Some(50), false, false),
                &ops,
                0.95,
                true,
                Route::Review(vec!["low_confidence"]),
            ),
            (
                "ambiguous",
                choice(Some(90), true, false),
                &ops,
                0.95,
                false,
                Route::Review(vec!["ambiguous"]),
            ),
            (
                "linked with an unexpected status",
                choice(Some(80), false, true),
                &unexpected,
                0.95,
                false,
                Route::Review(vec!["unexpected_status"]),
            ),
            (
                "every reason, in order",
                choice(Some(60), true, false),
                &unexpected,
                0.5,
                false,
                Route::Review(vec!["low_confidence", "ambiguous", "unexpected_status"]),
            ),
            (
                "a weak candidate without ops",
                choice(Some(60), false, false),
                &empty,
                0.9,
                true,
                Route::Unmatched,
            ),
            (
                "no candidate, a company",
                choice(None, false, false),
                &empty,
                0.9,
                true,
                Route::NewApplication,
            ),
            (
                "no candidate, no company",
                choice(None, false, false),
                &empty,
                0.9,
                false,
                Route::Unmatched,
            ),
        ];
        for (name, choice, plan, confidence, has_company, want) in cases {
            assert_eq!(
                route(&choice, plan, confidence, has_company),
                want,
                "{name}"
            );
        }
    }
}
