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

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

use crate::applications::stages::MAX_STAGES;
use crate::applications::timeline::{self, EventKind};
use crate::applications::{self, Application, ApplicationStatus, MAX_SHORT};
use crate::email::heuristics;
use crate::events::Event;
use crate::runs::approvals::DecisionError;
use crate::runs::review::ReviewItem;
use crate::runs::{REVIEW_THRESHOLD, StepContext, StepError, Workflow};
use crate::tools::{Permission, Tool, ToolContext, ToolError, UndoError};
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

/// Why an apply helper changed nothing.
#[derive(Debug)]
pub(crate) enum ApplyError {
    /// The application doesn't exist.
    Missing,
    /// Its status isn't the one the change starts from.
    Changed(ApplicationStatus),
    /// It already has [`MAX_STAGES`] stages.
    TooManyStages,
    /// The stage name is blank or longer than 200 characters.
    BadName,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ApplyError {
    fn from(err: sqlx::Error) -> Self {
        Self::Db(err)
    }
}

/// What `set_application_status`'s undo needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StatusUndo {
    application_id: i64,
    event_id: i64,
    from_status: ApplicationStatus,
    to_status: ApplicationStatus,
    previous_status_changed_at: DateTime<Utc>,
}

/// What `add_application_stage`'s undo needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StageUndo {
    application_id: i64,
    stage_id: i64,
    event_id: i64,
}

/// What [`apply_status`] did.
#[derive(Debug)]
pub(crate) struct StatusApplied {
    pub event_id: i64,
    pub changed: bool,
    undo: Option<StatusUndo>,
}

/// What [`apply_stage`] did.
#[derive(Debug)]
pub(crate) struct StageApplied {
    pub event_id: Option<i64>,
    pub stage_id: Option<i64>,
    pub changed: bool,
    undo: Option<StageUndo>,
}

/// Locks application `id` for the rest of the transaction and returns its status and
/// `status_changed_at`.
async fn lock(
    conn: &mut PgConnection,
    id: i64,
) -> Result<(ApplicationStatus, DateTime<Utc>), ApplyError> {
    sqlx::query_as("SELECT status, status_changed_at FROM applications WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or(ApplyError::Missing)
}

/// The newest `kind` event of application `id` that email `source_email_id` caused.
async fn sourced_event(
    conn: &mut PgConnection,
    id: i64,
    kind: EventKind,
    source_email_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM application_events WHERE application_id = $1 AND kind = $2 \
         AND source_email_id = $3 ORDER BY id DESC LIMIT 1",
    )
    .bind(id)
    .bind(kind.as_str())
    .bind(source_email_id)
    .fetch_optional(conn)
    .await
}

/// Moves application `id` from `expected` to `to` because of email `source_email_id`, under
/// the application lock: sets `status`, `status_changed_at` and `updated_at` and records a
/// sourced `status_changed` event. When the email already caused a status change there,
/// returns that event unchanged instead, so an email never applies twice. A status other
/// than `expected` is [`ApplyError::Changed`]. The tool and accept both use it.
pub(crate) async fn apply_status(
    conn: &mut PgConnection,
    id: i64,
    expected: ApplicationStatus,
    to: ApplicationStatus,
    source_email_id: i64,
) -> Result<StatusApplied, ApplyError> {
    let (current, changed_at) = lock(conn, id).await?;
    if let Some(event_id) =
        sourced_event(conn, id, EventKind::StatusChanged, source_email_id).await?
    {
        return Ok(StatusApplied {
            event_id,
            changed: false,
            undo: None,
        });
    }
    if current != expected || expected == to {
        return Err(ApplyError::Changed(current));
    }
    sqlx::query(
        "UPDATE applications SET status = $2, status_changed_at = now(), updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(to.as_str())
    .execute(&mut *conn)
    .await?;
    let event = timeline::record_sourced(
        conn,
        id,
        EventKind::StatusChanged,
        Some(expected),
        Some(to),
        None,
        Some(source_email_id),
    )
    .await?;
    Ok(StatusApplied {
        event_id: event.id,
        changed: true,
        undo: Some(StatusUndo {
            application_id: id,
            event_id: event.id,
            from_status: expected,
            to_status: to,
            previous_status_changed_at: changed_at,
        }),
    })
}

/// Appends stage `name` to application `id` because of email `source_email_id`, under the
/// application lock, and records a sourced `stage_added` event. Changes nothing (returning
/// `changed: false`) when the email already added a stage there or a stage with that name
/// (trimmed, any case) exists. The tool and accept both use it.
pub(crate) async fn apply_stage(
    conn: &mut PgConnection,
    id: i64,
    name: &str,
    source_email_id: i64,
) -> Result<StageApplied, ApplyError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_SHORT {
        return Err(ApplyError::BadName);
    }
    lock(conn, id).await?;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM application_stages WHERE application_id = $1 \
         AND lower(btrim(name)) = lower($2) ORDER BY position LIMIT 1",
    )
    .bind(id)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(event_id) = sourced_event(conn, id, EventKind::StageAdded, source_email_id).await? {
        return Ok(StageApplied {
            event_id: Some(event_id),
            stage_id: existing,
            changed: false,
            undo: None,
        });
    }
    if existing.is_some() {
        return Ok(StageApplied {
            event_id: None,
            stage_id: existing,
            changed: false,
            undo: None,
        });
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_stages WHERE application_id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    if count >= MAX_STAGES {
        return Err(ApplyError::TooManyStages);
    }
    let stage_id: i64 = sqlx::query_scalar(
        "INSERT INTO application_stages (application_id, name, position) VALUES ($1, $2, $3) \
         RETURNING id",
    )
    .bind(id)
    .bind(name)
    .bind(i32::try_from(count).unwrap_or(i32::MAX))
    .fetch_one(&mut *conn)
    .await?;
    let event = timeline::record_sourced(
        conn,
        id,
        EventKind::StageAdded,
        None,
        None,
        Some(name),
        Some(source_email_id),
    )
    .await?;
    sqlx::query("UPDATE applications SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(StageApplied {
        event_id: Some(event.id),
        stage_id: Some(stage_id),
        changed: true,
        undo: Some(StageUndo {
            application_id: id,
            stage_id,
            event_id: event.id,
        }),
    })
}

/// A tool failure for an [`ApplyError`]. A database error goes to the log rather than to
/// the caller (its text can name tables).
fn apply_failed(id: i64, what: &'static str) -> impl FnOnce(ApplyError) -> ToolError {
    move |err| {
        ToolError::Failed(match err {
            ApplyError::Missing => format!("application {id} doesn't exist"),
            ApplyError::Changed(status) => format!(
                "application {id} is {} now, not the status this change starts from",
                status.as_str()
            ),
            ApplyError::TooManyStages => format!("application {id} already has 50 stages"),
            ApplyError::BadName => "the stage name must be 1 to 200 characters".into(),
            ApplyError::Db(err) => {
                rocket::warn!("match_email: cannot {what}: {err}");
                format!("cannot {what}")
            }
        })
    }
}

fn db_failed(what: &'static str) -> impl FnOnce(sqlx::Error) -> ToolError {
    move |err| {
        rocket::warn!("match_email: cannot {what}: {err}");
        ToolError::Failed(format!("cannot {what}"))
    }
}

fn status_schema(description: &str) -> Value {
    let names: Vec<&str> = ApplicationStatus::ALL
        .iter()
        .map(|status| status.as_str())
        .collect();
    json!({"type": "string", "enum": names, "description": description})
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusInput {
    pub application_id: i64,
    pub status: ApplicationStatus,
    pub expected_status: ApplicationStatus,
    pub source_email_id: i64,
}

#[derive(Debug, Serialize)]
pub struct StatusSet {
    pub event_id: i64,
    pub changed: bool,
}

/// `write`: moves an application to a new status because of an email. Undo restores the
/// previous status and `status_changed_at` and deletes the event, unless the status changed
/// since.
pub struct SetApplicationStatus;

#[rocket::async_trait]
impl Tool for SetApplicationStatus {
    const NAME: &'static str = "set_application_status";
    const DESCRIPTION: &'static str =
        "Moves an application from its expected status to a new one because of an email";
    const PERMISSION: Permission = Permission::Write;
    type Input = StatusInput;
    type Output = StatusSet;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "application_id": {"type": "integer", "description": "The application's id"},
                "status": status_schema("The new status"),
                "expected_status": status_schema("The status the change starts from"),
                "source_email_id": {"type": "integer", "description": "The email that caused it"}
            },
            "required": ["application_id", "status", "expected_status", "source_email_id"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"event_id": {"type": "integer"}, "changed": {"type": "boolean"}},
            "required": ["event_id", "changed"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        ctx: &mut ToolContext<'_>,
        input: StatusInput,
    ) -> Result<StatusSet, ToolError> {
        let id = input.application_id;
        let what = "set an application's status";
        let mut tx = ctx.pool().begin().await.map_err(db_failed(what))?;
        let applied = apply_status(
            &mut tx,
            id,
            input.expected_status,
            input.status,
            input.source_email_id,
        )
        .await
        .map_err(apply_failed(id, what))?;
        tx.commit().await.map_err(db_failed(what))?;
        if let Some(undo) = &applied.undo {
            ctx.set_undo(json!(undo));
        }
        Ok(StatusSet {
            event_id: applied.event_id,
            changed: applied.changed,
        })
    }

    async fn undo(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError> {
        let undo: StatusUndo = serde_json::from_value(data).map_err(|_| {
            UndoError::Conflict("the undo data doesn't fit set_application_status".into())
        })?;
        let id = undo.application_id;
        let current: Option<ApplicationStatus> =
            sqlx::query_scalar("SELECT status FROM applications WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(current) = current else {
            return Err(UndoError::Conflict(format!("application {id} is gone")));
        };
        if current != undo.to_status {
            return Err(UndoError::Conflict(format!(
                "application {id}'s status changed since"
            )));
        }
        let (found, later): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM application_events WHERE id = $2 \
               AND application_id = $1 AND kind = 'status_changed'), \
             EXISTS (SELECT 1 FROM application_events WHERE application_id = $1 \
               AND kind = 'status_changed' AND id > $2)",
        )
        .bind(id)
        .bind(undo.event_id)
        .fetch_one(&mut *conn)
        .await?;
        if !found {
            return Err(UndoError::Conflict(format!(
                "status event {} of application {id} is gone",
                undo.event_id
            )));
        }
        if later {
            return Err(UndoError::Conflict(format!(
                "application {id}'s status changed again since"
            )));
        }
        sqlx::query(
            "UPDATE applications SET status = $2, status_changed_at = $3, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(undo.from_status.as_str())
        .bind(undo.previous_status_changed_at)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM application_events WHERE id = $1")
            .bind(undo.event_id)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageInput {
    pub application_id: i64,
    pub name: String,
    pub source_email_id: i64,
}

#[derive(Debug, Serialize)]
pub struct StageAdded {
    /// The `stage_added` event; `null` when a stage with the name already existed.
    pub event_id: Option<i64>,
    pub stage_id: Option<i64>,
    pub changed: bool,
}

/// `write`: appends an interview stage to an application because of an email. Undo
/// deletes the stage and its event and closes the gap, unless the stage was completed.
pub struct AddApplicationStage;

#[rocket::async_trait]
impl Tool for AddApplicationStage {
    const NAME: &'static str = "add_application_stage";
    const DESCRIPTION: &'static str =
        "Appends an interview stage to an application because of an email";
    const PERMISSION: Permission = Permission::Write;
    type Input = StageInput;
    type Output = StageAdded;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "application_id": {"type": "integer", "description": "The application's id"},
                "name": {"type": "string", "description": "The stage's name"},
                "source_email_id": {"type": "integer", "description": "The email that caused it"}
            },
            "required": ["application_id", "name", "source_email_id"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "event_id": {"type": ["integer", "null"]},
                "stage_id": {"type": ["integer", "null"]},
                "changed": {"type": "boolean"}
            },
            "required": ["event_id", "stage_id", "changed"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        ctx: &mut ToolContext<'_>,
        input: StageInput,
    ) -> Result<StageAdded, ToolError> {
        let id = input.application_id;
        let what = "add a stage";
        let mut tx = ctx.pool().begin().await.map_err(db_failed(what))?;
        let applied = apply_stage(&mut tx, id, &input.name, input.source_email_id)
            .await
            .map_err(apply_failed(id, what))?;
        tx.commit().await.map_err(db_failed(what))?;
        if let Some(undo) = &applied.undo {
            ctx.set_undo(json!(undo));
        }
        Ok(StageAdded {
            event_id: applied.event_id,
            stage_id: applied.stage_id,
            changed: applied.changed,
        })
    }

    async fn undo(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError> {
        let undo: StageUndo = serde_json::from_value(data).map_err(|_| {
            UndoError::Conflict("the undo data doesn't fit add_application_stage".into())
        })?;
        let id = undo.application_id;
        let found: Option<i64> =
            sqlx::query_scalar("SELECT id FROM applications WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        if found.is_none() {
            return Err(UndoError::Conflict(format!("application {id} is gone")));
        }
        let position: Option<i32> = sqlx::query_scalar(
            "DELETE FROM application_stages WHERE id = $1 AND application_id = $2 \
             AND completed_at IS NULL RETURNING position",
        )
        .bind(undo.stage_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(position) = position else {
            return Err(UndoError::Conflict(format!(
                "stage {} of application {id} is gone or completed",
                undo.stage_id
            )));
        };
        sqlx::query(
            "UPDATE application_stages SET position = position - 1 \
             WHERE application_id = $1 AND position > $2",
        )
        .bind(id)
        .bind(position)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "DELETE FROM application_events WHERE id = $1 AND application_id = $2 \
             AND kind = 'stage_added'",
        )
        .bind(undo.event_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
        sqlx::query("UPDATE applications SET updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }
}

/// The run output, and what a finished match row reads back as. Every key is always
/// present.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
struct Outcome {
    email_id: Option<i64>,
    classification_id: i64,
    status: String,
    /// The linked (or applied) application.
    application_id: Option<i64>,
    /// The top candidate's score.
    confidence: Option<f64>,
    reasons: Value,
    /// The applied ops for `applied`, else the planned ones.
    ops: Value,
    review_item_id: Option<i64>,
    review_kind: Option<String>,
}

impl Outcome {
    fn skipped(classification_id: i64, email_id: Option<i64>) -> Self {
        Self {
            email_id,
            classification_id,
            status: "skipped".into(),
            application_id: None,
            confidence: None,
            reasons: json!([]),
            ops: json!([]),
            review_item_id: None,
            review_kind: None,
        }
    }

    fn json(&self) -> Value {
        json!(self)
    }
}

/// Classification `classification_id`'s match row, as an [`Outcome`].
async fn stored(pool: &PgPool, classification_id: i64) -> Result<Option<Outcome>, sqlx::Error> {
    sqlx::query_as(
        "SELECT m.email_id, m.classification_id, m.status, m.application_id, m.confidence, \
         m.reasons, m.ops, m.review_item_id, r.kind AS review_kind \
         FROM email_matches m LEFT JOIN review_items r ON r.id = m.review_item_id \
         WHERE m.classification_id = $1",
    )
    .bind(classification_id)
    .fetch_optional(pool)
    .await
}

/// The classification and email columns the step reads.
#[derive(Debug, sqlx::FromRow)]
struct Classified {
    id: i64,
    email_id: i64,
    status: String,
    class: Option<String>,
    confidence: Option<f64>,
    company: Option<String>,
    role: Option<String>,
    stage: Option<String>,
    thread_id: i64,
    subject: String,
    from_name: Option<String>,
    from_address: Option<String>,
    sent_at: Option<DateTime<Utc>>,
    ats: Option<String>,
    /// The email's newest `classified` row.
    newest: Option<i64>,
}

impl Classified {
    fn email(&self) -> Value {
        json!({
            "subject": self.subject,
            "from_name": self.from_name,
            "from_address": self.from_address,
            "sent_at": self.sent_at,
        })
    }
}

fn db(what: &'static str) -> impl FnOnce(sqlx::Error) -> StepError {
    move |err| StepError::Retry(format!("cannot {what}: {err}"))
}

/// The run's input.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunInput {
    classification_id: i64,
}

/// What earlier links say about each application, for an email of `thread_id`.
async fn histories(
    pool: &PgPool,
    email_id: i64,
    thread_id: i64,
) -> Result<HashMap<i64, History>, sqlx::Error> {
    let links: Vec<(i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT m.application_id, e.thread_id, e.from_address FROM email_matches m \
         JOIN emails e ON e.id = m.email_id \
         WHERE m.status IN ('applied', 'no_action') AND m.application_id IS NOT NULL \
         AND m.email_id <> $1",
    )
    .bind(email_id)
    .fetch_all(pool)
    .await?;
    let mut by_application: HashMap<i64, History> = HashMap::new();
    for (application_id, thread, from_address) in links {
        let history = by_application.entry(application_id).or_default();
        history.thread |= thread == thread_id;
        if let Some(domain) = from_address.as_deref().and_then(heuristics::domain) {
            history.domains.insert(domain.trim().to_lowercase());
        }
    }
    Ok(by_application)
}

/// The `match` step; see the module docs.
async fn match_step(ctx: &mut StepContext<'_>, input: Value) -> Result<Value, StepError> {
    let input = RunInput::deserialize(&input)
        .map_err(|err| StepError::Fail(format!("invalid input: {err}")))?;
    let pool = ctx.pool().clone();
    let classification_id = input.classification_id;
    let found: Option<Classified> = sqlx::query_as(
        "SELECT c.id, c.email_id, c.status, c.class, c.confidence, c.company, c.role, c.stage, \
         e.thread_id, e.subject, e.from_name, e.from_address, e.sent_at, e.ats, \
         (SELECT max(n.id) FROM email_classifications n \
           WHERE n.email_id = c.email_id AND n.status = 'classified') AS newest \
         FROM email_classifications c JOIN emails e ON e.id = c.email_id WHERE c.id = $1",
    )
    .bind(classification_id)
    .fetch_optional(&pool)
    .await
    .map_err(db("read the classification"))?;
    let Some(source) = found else {
        return Ok(Outcome::skipped(classification_id, None).json());
    };
    let class = source.class.as_deref().and_then(EmailClass::parse);
    let (Some(class), Some(confidence)) = (class, source.confidence) else {
        return Ok(Outcome::skipped(classification_id, Some(source.email_id)).json());
    };
    if source.status != "classified" || source.newest != Some(source.id) {
        return Ok(Outcome::skipped(classification_id, Some(source.email_id)).json());
    }

    sqlx::query(
        "INSERT INTO email_matches (classification_id, email_id, run_id) VALUES ($1, $2, $3) \
         ON CONFLICT (classification_id) DO NOTHING",
    )
    .bind(classification_id)
    .bind(source.email_id)
    .bind(ctx.run_id())
    .execute(&pool)
    .await
    .map_err(db("store the match"))?;
    let row = stored(&pool, classification_id)
        .await
        .map_err(db("read the match"))?
        .ok_or_else(|| StepError::Retry("the match row is gone".into()))?;
    if row.status != "pending" {
        return Ok(row.json());
    }

    let apps: Vec<(i64, String, String, ApplicationStatus)> =
        sqlx::query_as("SELECT id, company, title, status FROM applications ORDER BY id")
            .fetch_all(&pool)
            .await
            .map_err(db("read the applications"))?;
    let stage_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT application_id, name FROM application_stages ORDER BY application_id, position",
    )
    .fetch_all(&pool)
    .await
    .map_err(db("read the stages"))?;
    let mut stages: HashMap<i64, Vec<String>> = HashMap::new();
    for (application_id, name) in stage_rows {
        stages.entry(application_id).or_default().push(name);
    }
    let histories = histories(&pool, source.email_id, source.thread_id)
        .await
        .map_err(db("read earlier matches"))?;

    let facts = EmailFacts {
        company: source.company.as_deref(),
        role: source.role.as_deref(),
        from_name: source.from_name.as_deref(),
        from_address: source.from_address.as_deref(),
    };
    let none = History::default();
    let scored = apps
        .into_iter()
        .map(|(id, company, title, status)| {
            let app = AppFacts {
                id,
                company,
                title,
                status,
            };
            let scored = score(&facts, &app, histories.get(&id).unwrap_or(&none));
            (app, scored)
        })
        .collect();
    let choice = choose(scored);
    let plan = choice
        .top()
        .map(|top| {
            let names = stages
                .get(&top.application_id)
                .map_or(&[][..], Vec::as_slice);
            plan(class, top.status, names, source.stage.as_deref())
        })
        .unwrap_or_default();
    let routed = route(&choice, &plan, confidence, source.company.is_some());

    let top = choice.top();
    let mut outcome = Outcome {
        email_id: Some(source.email_id),
        classification_id,
        status: String::new(),
        application_id: top.filter(|_| choice.linked).map(|top| top.application_id),
        confidence: top.map(|top| top.score),
        reasons: json!(top.map(|top| top.reasons.clone()).unwrap_or_default()),
        ops: json!(plan.ops),
        review_item_id: None,
        review_kind: None,
    };
    let review = |err: sqlx::Error| StepError::Retry(format!("cannot store a review item: {err}"));
    match (&routed, top) {
        (Route::Apply, Some(top)) => {
            for op in &plan.ops {
                let (tool, input) = match op {
                    Op::SetStatus { from, to } => (
                        SetApplicationStatus::NAME,
                        json!({"application_id": top.application_id, "status": to,
                               "expected_status": from, "source_email_id": source.email_id}),
                    ),
                    Op::AddStage { name } => (
                        AddApplicationStage::NAME,
                        json!({"application_id": top.application_id, "name": name,
                               "source_email_id": source.email_id}),
                    ),
                };
                ctx.tool(tool, input).await?;
            }
            outcome.status = "applied".into();
        }
        (Route::NoAction, _) => outcome.status = "no_action".into(),
        (Route::Review(review_reasons), Some(top)) => {
            let payload = json!({
                "email_id": source.email_id,
                "classification_id": classification_id,
                "class": class,
                "email": source.email(),
                "application_id": top.application_id,
                "company": top.company,
                "title": top.title,
                "ops": plan.ops,
                "reasons": top.reasons,
                "review_reasons": review_reasons,
                "candidates": choice.candidates,
            });
            let item_confidence = confidence.min(top.score);
            let id = ctx
                .review(MATCH_REVIEW_KIND, item_confidence, payload)
                .await
                .map_err(review)?;
            outcome.status = "review".into();
            outcome.review_item_id = Some(id);
            outcome.review_kind = Some(MATCH_REVIEW_KIND.into());
        }
        (Route::NewApplication, _) => {
            let company = source.company.as_deref().unwrap_or_default();
            let proposed = proposal(class, company, source.role.as_deref(), source.ats.is_some());
            let payload = json!({
                "email_id": source.email_id,
                "classification_id": classification_id,
                "class": class,
                "email": source.email(),
                "application": proposed,
            });
            let id = ctx
                .review(NEW_APPLICATION_REVIEW_KIND, confidence, payload)
                .await
                .map_err(review)?;
            outcome.status = "review".into();
            outcome.review_item_id = Some(id);
            outcome.review_kind = Some(NEW_APPLICATION_REVIEW_KIND.into());
        }
        _ => outcome.status = "unmatched".into(),
    }

    let updated = sqlx::query(
        "UPDATE email_matches SET status = $2, application_id = $3, confidence = $4, \
         reasons = $5, candidates = $6, ops = $7, review_item_id = $8, run_id = $9, \
         updated_at = now() WHERE classification_id = $1 AND status = 'pending'",
    )
    .bind(classification_id)
    .bind(&outcome.status)
    .bind(outcome.application_id)
    .bind(outcome.confidence)
    .bind(&outcome.reasons)
    .bind(json!(choice.candidates))
    .bind(&outcome.ops)
    .bind(outcome.review_item_id)
    .bind(ctx.run_id())
    .execute(&pool)
    .await
    .map_err(db("store the match"))?;
    if updated.rows_affected() == 0 {
        // Another attempt or an accept finished it first.
        if let Some(row) = stored(&pool, classification_id)
            .await
            .map_err(db("read the match"))?
        {
            return Ok(row.json());
        }
    }

    if routed == Route::Apply
        && let Some(id) = outcome.application_id
    {
        publish_updated(ctx, &pool, id).await;
    }
    Ok(outcome.json())
}

/// Publishes `application.updated` with application `id`'s current row. Best effort.
async fn publish_updated(ctx: &StepContext<'_>, pool: &PgPool, id: i64) {
    let found = sqlx::query_as::<_, Application>(&format!(
        "SELECT {} FROM applications WHERE id = $1",
        applications::COLUMNS
    ))
    .bind(id)
    .fetch_optional(pool)
    .await;
    match found {
        Ok(Some(application)) => ctx.bus().publish(Event::ApplicationUpdated { application }),
        Ok(None) => {}
        Err(err) => rocket::warn!("match_email: cannot publish application {id}: {err}"),
    }
}

/// The workflow. Its input is `{"classification_id"}`.
pub struct MatchEmail;

#[rocket::async_trait]
impl Workflow for MatchEmail {
    fn name(&self) -> &'static str {
        WORKFLOW
    }

    fn steps(&self) -> &'static [&'static str] {
        &[STEP]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        match step {
            STEP => match_step(ctx, input).await,
            other => Err(StepError::Fail(format!("no step is named {other:?}"))),
        }
    }
}

/// The parts of an `email.match` payload accepting reads.
#[derive(Debug, Deserialize)]
struct MatchPayload {
    email_id: i64,
    classification_id: i64,
    application_id: i64,
    ops: Vec<Op>,
}

/// The parts of an `email.new_application` payload accepting reads.
#[derive(Debug, Deserialize)]
struct NewApplicationPayload {
    email_id: i64,
    classification_id: i64,
    application: Proposal,
}

/// What accepting an email review item changed.
#[derive(Debug)]
pub(crate) struct Accepted {
    pub application: Application,
    /// The application is new (`email.new_application`).
    pub created: bool,
}

fn accept_conflict(err: ApplyError) -> DecisionError {
    match err {
        ApplyError::Db(err) => DecisionError::Db(err),
        ApplyError::Missing => DecisionError::Conflict("the application is gone".into()),
        ApplyError::Changed(status) => DecisionError::Conflict(format!(
            "the application is {} now, not the status the change starts from",
            status.as_str()
        )),
        ApplyError::TooManyStages => {
            DecisionError::Conflict("the application already has 50 stages".into())
        }
        ApplyError::BadName => DecisionError::Conflict("the stage name is invalid".into()),
    }
}

/// Marks classification `classification_id`'s match `applied` to `application_id`.
async fn link(
    conn: &mut PgConnection,
    classification_id: i64,
    application_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE email_matches SET status = 'applied', application_id = $2, updated_at = now() \
         WHERE classification_id = $1",
    )
    .bind(classification_id)
    .bind(application_id)
    .execute(conn)
    .await
    .map(|_| ())
}

/// Applies a pending `email.match` or `email.new_application` item inside the caller's
/// transaction, as the user's own change (no action, no undo): the match's ops with the
/// email as their source, or the proposed application with a sourced `created` event. Its
/// match row becomes `applied`. Any refusal is a [`DecisionError::Conflict`], and the
/// caller rolls back.
pub(crate) async fn accept(
    conn: &mut PgConnection,
    item: &ReviewItem,
) -> Result<Accepted, DecisionError> {
    let unreadable = |_| DecisionError::Conflict(format!("review item {} doesn't parse", item.id));
    match item.kind.as_str() {
        MATCH_REVIEW_KIND => {
            let payload = MatchPayload::deserialize(&item.payload).map_err(unreadable)?;
            let id = payload.application_id;
            lock(conn, id).await.map_err(accept_conflict)?;
            for op in &payload.ops {
                match op {
                    Op::SetStatus { from, to } => {
                        apply_status(conn, id, *from, *to, payload.email_id)
                            .await
                            .map_err(accept_conflict)?;
                    }
                    Op::AddStage { name } => {
                        apply_stage(conn, id, name, payload.email_id)
                            .await
                            .map_err(accept_conflict)?;
                    }
                }
            }
            link(conn, payload.classification_id, id).await?;
            let application = sqlx::query_as(&format!(
                "SELECT {} FROM applications WHERE id = $1",
                applications::COLUMNS
            ))
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
            Ok(Accepted {
                application,
                created: false,
            })
        }
        NEW_APPLICATION_REVIEW_KIND => {
            let payload = NewApplicationPayload::deserialize(&item.payload).map_err(unreadable)?;
            let Proposal {
                company,
                title,
                status,
            } = payload.application;
            let fits = |text: &str| {
                let text = text.trim();
                !text.is_empty() && text.chars().count() <= MAX_SHORT
            };
            if !fits(&company) || !fits(&title) {
                return Err(DecisionError::Conflict(format!(
                    "review item {} proposes an invalid application",
                    item.id
                )));
            }
            let application: Application = sqlx::query_as(&format!(
                "INSERT INTO applications (company, title, status, source) \
                 VALUES ($1, $2, $3, 'email') RETURNING {}",
                applications::COLUMNS
            ))
            .bind(company.trim())
            .bind(title.trim())
            .bind(status.as_str())
            .fetch_one(&mut *conn)
            .await?;
            timeline::record_sourced(
                conn,
                application.id,
                EventKind::Created,
                None,
                Some(status),
                None,
                Some(payload.email_id),
            )
            .await?;
            link(conn, payload.classification_id, application.id).await?;
            Ok(Accepted {
                application,
                created: true,
            })
        }
        other => Err(DecisionError::Conflict(format!(
            "review items of kind {other:?} can't be accepted"
        ))),
    }
}

/// A `match_email` eval case's `setup`: the applications, then the emails to ingest in
/// order with their stored classifications. The last email is the one the case matches.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalSetup {
    #[serde(default)]
    applications: Vec<EvalApplication>,
    emails: Vec<EvalEmail>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalApplication {
    /// Its id is the variable `application_<name>`.
    name: String,
    company: String,
    title: String,
    status: ApplicationStatus,
    #[serde(default)]
    stages: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalEmail {
    /// Relative to `api/fixtures/`.
    file: String,
    classification: EvalClassification,
    /// An application name: the email is stored as already matched (`applied`) to it.
    #[serde(default)]
    matched_to: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalClassification {
    class: String,
    confidence: f64,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    stage: Option<String>,
}

impl EvalSetup {
    /// Refuses a setup the seeding can't honour, before anything is written.
    fn check(&self) -> Result<(), String> {
        if self.emails.is_empty() {
            return Err("the setup lists no emails".into());
        }
        let mut names = BTreeSet::new();
        for app in &self.applications {
            if app.name.trim().is_empty() || !names.insert(app.name.as_str()) {
                return Err(format!(
                    "application name {:?} is blank or repeated",
                    app.name
                ));
            }
        }
        for email in &self.emails {
            crate::workflows::classify_email::check_eval_path(&email.file)?;
            let classification = &email.classification;
            if EmailClass::parse(&classification.class).is_none() {
                return Err(format!("{}: unknown class", email.file));
            }
            if !(0.0..=1.0).contains(&classification.confidence) {
                return Err(format!("{}: confidence must be 0 to 1", email.file));
            }
            if let Some(name) = &email.matched_to
                && !names.contains(name.as_str())
            {
                return Err(format!("{}: no application is named {name:?}", email.file));
            }
        }
        Ok(())
    }
}

/// The eval suite for `match_email` (dataset `api/evals/match_email.json`). Setup clears
/// matches, classifications, emails, threads and applications; inserts the case's
/// applications with their stages and `created` events; ingests each email without queuing
/// a run and stores a `classified` row for it (plus an `applied` match row for
/// `matched_to`). It makes no model call. Variables: `application_<name>` per application,
/// and `email_id` and `classification_id` of the last email.
pub struct MatchEmailEval;

#[rocket::async_trait]
impl crate::evals::Suite for MatchEmailEval {
    fn workflow(&self) -> &'static str {
        WORKFLOW
    }

    async fn setup(
        &self,
        pool: &PgPool,
        setup: &Value,
    ) -> Result<serde_json::Map<String, Value>, String> {
        use crate::workflows::classify_email::{
            CLASSIFIER_VERSION, current_key, ingest_eval_fixture,
        };
        let setup = EvalSetup::deserialize(setup).map_err(|err| format!("invalid setup: {err}"))?;
        setup.check()?;
        for table in [
            "email_matches",
            "email_classifications",
            "emails",
            "email_threads",
            "application_events",
            "application_stages",
            "applications",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(pool)
                .await
                .map_err(|err| format!("cannot clear {table}: {err}"))?;
        }

        let mut vars = serde_json::Map::new();
        let mut ids = HashMap::new();
        for app in &setup.applications {
            let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO applications (company, title, status) VALUES ($1, $2, $3) \
                 RETURNING id",
            )
            .bind(&app.company)
            .bind(&app.title)
            .bind(app.status.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(|err| format!("cannot insert application {:?}: {err}", app.name))?;
            timeline::record(
                &mut tx,
                id,
                EventKind::Created,
                None,
                Some(app.status),
                None,
                None,
            )
            .await
            .map_err(|err| format!("cannot record application {:?}: {err}", app.name))?;
            for (position, name) in app.stages.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO application_stages (application_id, name, position) \
                     VALUES ($1, $2, $3)",
                )
                .bind(id)
                .bind(name)
                .bind(i32::try_from(position).unwrap_or(i32::MAX))
                .execute(&mut *tx)
                .await
                .map_err(|err| format!("cannot insert stage {name:?}: {err}"))?;
            }
            tx.commit().await.map_err(|err| err.to_string())?;
            ids.insert(app.name.as_str(), id);
            vars.insert(format!("application_{}", app.name), json!(id));
        }

        let mut last = None;
        for email in &setup.emails {
            let email_id = ingest_eval_fixture(pool, &email.file).await?;
            let key = current_key(pool, email_id)
                .await
                .map_err(|err| format!("cannot read {}: {err}", email.file))?;
            let answer = &email.classification;
            let classification_id: i64 = sqlx::query_scalar(
                "INSERT INTO email_classifications (email_id, input_key, classifier_version, \
                 status, class, confidence, company, role, stage) \
                 VALUES ($1, $2, $3, 'classified', $4, $5, $6, $7, $8) RETURNING id",
            )
            .bind(email_id)
            .bind(&key)
            .bind(CLASSIFIER_VERSION)
            .bind(&answer.class)
            .bind(answer.confidence)
            .bind(&answer.company)
            .bind(&answer.role)
            .bind(&answer.stage)
            .fetch_one(pool)
            .await
            .map_err(|err| format!("cannot classify {}: {err}", email.file))?;
            if let Some(name) = &email.matched_to {
                sqlx::query(
                    "INSERT INTO email_matches (classification_id, email_id, status, \
                     application_id) VALUES ($1, $2, 'applied', $3)",
                )
                .bind(classification_id)
                .bind(email_id)
                .bind(ids.get(name.as_str()))
                .execute(pool)
                .await
                .map_err(|err| format!("cannot match {}: {err}", email.file))?;
            }
            last = Some((email_id, classification_id));
        }
        if let Some((email_id, classification_id)) = last {
            vars.insert("email_id".into(), json!(email_id));
            vars.insert("classification_id".into(), json!(classification_id));
        }
        Ok(vars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evals;
    use crate::workflows::classify_email::EVAL_FIXTURES;
    use ApplicationStatus::{Applied, Interviewing, Offer, Rejected, Sourced};

    #[test]
    fn match_email_dataset_parses() {
        let dataset = evals::load(&evals::default_path(WORKFLOW), WORKFLOW)
            .expect("the shipped dataset loads");
        assert!(dataset.cases.len() >= 25, "{}", dataset.cases.len());
        let mut statuses = BTreeSet::new();
        for case in &dataset.cases {
            let setup = EvalSetup::deserialize(&case.setup)
                .unwrap_or_else(|err| panic!("{}: invalid setup: {err}", case.name));
            setup
                .check()
                .unwrap_or_else(|err| panic!("{}: {err}", case.name));
            for email in &setup.emails {
                assert!(
                    std::path::Path::new(EVAL_FIXTURES)
                        .join(&email.file)
                        .is_file(),
                    "{}: {} is missing",
                    case.name,
                    email.file
                );
            }
            assert_eq!(
                case.input,
                json!({"classification_id": "${classification_id}"}),
                "{}",
                case.name
            );
            let output = case
                .expect
                .output
                .as_ref()
                .unwrap_or_else(|| panic!("{}: no output labels", case.name));
            assert!(
                output.get("application_id").is_some(),
                "{}: no application_id label",
                case.name
            );
            let status = output["status"]
                .as_str()
                .unwrap_or_else(|| panic!("{}: no status label", case.name));
            statuses.insert(status.to_owned());
        }
        assert_eq!(
            statuses.into_iter().collect::<Vec<_>>(),
            ["applied", "no_action", "review", "unmatched"],
            "every outcome has a case"
        );
    }

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
        /// (case, scored, candidate ids, ambiguous, linked)
        type Case = (&'static str, Vec<(AppFacts, Scored)>, Vec<i64>, bool, bool);
        let cases: Vec<Case> = vec![
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
        /// (case, class, status, stages, stated stage, ops)
        type StageCase<'a> = (
            &'a str,
            EmailClass,
            ApplicationStatus,
            &'a [String],
            Option<&'a str>,
            Vec<Op>,
        );
        let stage_cases: Vec<StageCase> = vec![
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
