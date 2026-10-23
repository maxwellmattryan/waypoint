//! Goals: the user's job-search request in their own words, the plan the `plan_goal`
//! workflow ([`crate::workflows::plan_goal`]) makes of it, and the plan's execution by
//! `plan_step` jobs ([`exec`]). See docs/goals.md.
//!
//! Every plan write is a new version of the goal's plan ([`store::write_version`]); the
//! newest is the current one. Every version has the same five steps, run in order. The
//! routes here create a goal, answer its clarifying questions, edit its plan (no model
//! call), run it and ask for a replan. Each locks the goal row while it checks the status
//! and writes, and publishes `goal.updated` once its write has committed.

pub mod exec;
pub mod store;

use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{self, Responder};
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};

use crate::applications::{
    ApiError, FieldError, JsonContent, REQUIRED, ValidationErrors, internal, parsed, text,
};
use crate::events::{Event, EventBus};
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::Wake;
use crate::runs::{self, RunStatus};
use crate::sourcing::{BAD_BOARD, BAD_SOURCE, Source, is_board_slug};
use crate::workflows::plan_goal;
use crate::workflows::prep_packet::PacketStatus;
use crate::workflows::score_fit::FitBand;

/// Most characters in a goal's text.
pub const MAX_GOAL: usize = 1_000;
/// Most characters in one answer to a clarifying question.
pub const MAX_ANSWER: usize = 1_000;
/// Most characters in a replan instruction.
pub const MAX_INSTRUCTION: usize = 1_000;
/// Most goals `GET /goals` returns.
pub const MAX_GOALS: i64 = 200;
/// Most target companies in a plan.
pub const MAX_COMPANIES: usize = 20;
/// Most characters in a company's name.
pub const MAX_NAME: usize = 100;
/// Most characters kept of a company's reason; longer ones are cut.
pub const MAX_REASON: usize = 300;
/// Most title keywords in a plan.
pub const MAX_KEYWORDS: usize = 10;
/// Most characters in one title keyword.
pub const MAX_KEYWORD: usize = 50;
/// Most locations in a plan.
pub const MAX_LOCATIONS: usize = 10;
/// Most characters in one location.
pub const MAX_LOCATION: usize = 100;
/// The largest `max_to_score`.
pub const MAX_TO_SCORE: i64 = 50;
/// The largest `shortlist_size`.
pub const MAX_SHORTLIST: i64 = 20;
/// Most characters in a plan's prep notes.
pub const MAX_PREP_NOTES: usize = 1_000;
/// Automatic replans after failed steps, per Run click.
pub const MAX_AUTO_REPLANS: i32 = 2;

const TOO_MANY_COMPANIES: &str = "At most 20 companies";
const DUPLICATE_BOARD: &str = "Duplicate board";
const BAD_MAX_TO_SCORE: &str = "Must be 1 to 50";
const BAD_SHORTLIST_SIZE: &str = "Must be 1 to 20";
const BAD_MIN_SCORE: &str = "Must be 0 to 100";
const NOT_OPEN_QUESTION: &str = "Not an open question of this goal";
const DUPLICATE_QUESTION: &str = "Answered twice";

/// Where a goal is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// A `plan_goal` run is queued, running or paused.
    Planning,
    /// Clarifying questions wait for the user's answers.
    NeedsAnswers,
    /// A current plan exists and isn't running.
    Planned,
    /// The plan is executing.
    Running,
    /// Every step of the current plan `succeeded` or was `skipped`.
    Completed,
    /// The planner run failed, or a step failed after the automatic replans ran out;
    /// `error` says why.
    Failed,
}

crate::memories::text_enum!(GoalStatus, "goal status", [
    Planning => "planning",
    NeedsAnswers => "needs_answers",
    Planned => "planned",
    Running => "running",
    Completed => "completed",
    Failed => "failed",
]);

/// Who wrote a plan version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanAuthor {
    Planner,
    User,
}

crate::memories::text_enum!(PlanAuthor, "plan author", [
    Planner => "planner",
    User => "user",
]);

/// Why a plan version was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanTrigger {
    /// The first plan for the goal (after any answers).
    Goal,
    /// An automatic replan after a step failed.
    StepFailed,
    /// The user asked for a replan.
    UserReplan,
    /// The user edited the plan.
    UserEdit,
}

crate::memories::text_enum!(PlanTrigger, "plan trigger", [
    Goal => "goal",
    StepFailed => "step_failed",
    UserReplan => "user_replan",
    UserEdit => "user_edit",
]);

/// Whether a plan version may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// It may run.
    Ready,
    /// A `plan_goal.plan` review item waits for the user.
    InReview,
    /// The user dismissed its review item.
    Dismissed,
}

crate::memories::text_enum!(PlanStatus, "plan status", [
    Ready => "ready",
    InReview => "in_review",
    Dismissed => "dismissed",
]);

/// What a plan step does. Every plan has all five, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepKind {
    TargetCompanies,
    Source,
    Score,
    Shortlist,
    Prep,
}

crate::memories::text_enum!(PlanStepKind, "plan step kind", [
    TargetCompanies => "target_companies",
    Source => "source",
    Score => "score",
    Shortlist => "shortlist",
    Prep => "prep",
]);

/// Where a plan step is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    /// Started, possibly waiting for its `source_board` jobs, `score_fit` runs or
    /// `prep_packet` runs.
    Running,
    Succeeded,
    Failed,
    /// Legacy: the `prep` placeholder before prep ran; never written now.
    Skipped,
}

crate::memories::text_enum!(PlanStepStatus, "plan step status", [
    Pending => "pending",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
    Skipped => "skipped",
]);

impl PlanStepStatus {
    /// `succeeded` or `skipped`.
    pub fn is_done(self) -> bool {
        matches!(self, Self::Succeeded | Self::Skipped)
    }
}

/// One `goals` row.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Goal {
    pub id: i64,
    /// The goal in the user's words.
    pub text: String,
    pub status: GoalStatus,
    /// Why the goal failed; `null` unless `failed`.
    #[schema(required = true)]
    pub error: Option<String>,
    /// Automatic replans since the last Run click.
    pub auto_replans: i32,
    /// The newest plan version.
    #[schema(required = true)]
    pub current_plan_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub(crate) const GOAL_COLUMNS: &str =
    "id, text, status, error, auto_replans, current_plan_id, created_at, updated_at";

/// One clarifying question.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct GoalQuestion {
    pub id: i64,
    /// Its position, from 1.
    pub ordinal: i32,
    pub question: String,
    /// The user's answer; `null` when skipped or not answered yet.
    #[schema(required = true)]
    pub answer: Option<String>,
    /// When the answers were submitted; `null` while the question is open.
    #[schema(required = true)]
    pub answered_at: Option<DateTime<Utc>>,
}

/// One target company, as stored in a plan's config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanCompanyConfig {
    /// 1–100 characters.
    pub name: String,
    pub source: Source,
    /// The board's slug, as in its board URL.
    pub board: String,
    /// Why this company; at most 300 characters.
    #[serde(default)]
    #[schema(required = true)]
    pub reason: Option<String>,
}

/// A validated plan: what every step runs on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanConfig {
    /// 1–20 companies, unique by `(source, board)`.
    pub companies: Vec<PlanCompanyConfig>,
    /// 1–10; a posting's title must contain one, ignoring case.
    pub title_keywords: Vec<String>,
    /// 0–10; when given, a posting's location must contain one, ignoring case.
    pub locations: Vec<String>,
    /// Only remote postings.
    pub remote_only: bool,
    /// 1–50: most postings scored.
    pub max_to_score: i32,
    /// 1–20: most postings shortlisted.
    pub shortlist_size: i32,
    /// 0–100: the lowest fit score shortlisted.
    pub min_score: i32,
    /// For the prep step; at most 1000 characters.
    pub prep_notes: String,
}

/// A company the planner gave that validation dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DroppedCompany {
    pub name: String,
    pub source: String,
    pub board: String,
    /// The rule it broke.
    pub why: String,
}

/// What validation dropped from the planner's answer; empty for user edits.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlanDropped {
    #[serde(default)]
    pub companies: Vec<DroppedCompany>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub locations: Vec<String>,
}

impl PlanDropped {
    pub fn any(&self) -> bool {
        !self.companies.is_empty() || !self.keywords.is_empty() || !self.locations.is_empty()
    }

    /// `{"companies": n, "keywords": n, "locations": n}`.
    pub fn counts(&self) -> Value {
        json!({
            "companies": self.companies.len(),
            "keywords": self.keywords.len(),
            "locations": self.locations.len(),
        })
    }
}

/// A target company as `GET /goals/{id}` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlanCompany {
    pub name: String,
    pub source: Source,
    pub board: String,
    #[schema(required = true)]
    pub reason: Option<String>,
    /// Whether Waypoint has sourced any posting from this board, computed when read.
    pub verified: bool,
}

/// One `plan_steps` row.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct PlanStep {
    pub id: i64,
    /// Its position, from 0.
    pub ordinal: i32,
    pub kind: PlanStepKind,
    pub status: PlanStepStatus,
    /// The step's inputs: cumulative, so a change to an earlier step's inputs changes it.
    pub config: Value,
    /// What the step found; `null` until it finishes.
    #[schema(required = true)]
    pub output: Option<Value>,
    /// Why it failed; `null` unless `failed`.
    #[schema(required = true)]
    pub last_error: Option<String>,
    /// The step of an earlier version this one was copied from.
    #[schema(required = true)]
    pub kept_from: Option<i64>,
    /// The version of `kept_from`'s plan.
    #[schema(required = true)]
    pub kept_from_version: Option<i32>,
    #[schema(required = true)]
    pub started_at: Option<DateTime<Utc>>,
    #[schema(required = true)]
    pub finished_at: Option<DateTime<Utc>>,
}

/// One shortlisted posting.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ShortlistEntry {
    /// 1-based.
    pub rank: i32,
    pub posting_id: i64,
    pub company: String,
    pub title: String,
    /// The fit score it was shortlisted with.
    pub score: i32,
    pub band: FitBand,
}

/// What the `prep` step did for one shortlisted posting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrepOutcome {
    /// A `prep_packet` run is queued or running; the step is still waiting.
    Queued,
    /// The step's run saved a packet.
    Prepared,
    /// The posting already had a `ready`, `in_review`, `approved` or `applied` packet.
    Reused,
    /// No packet: the posting is gone or unscored, or the run failed or saved none.
    Failed,
}

/// One posting of the `prep` step, with its packet's live status.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct PlanPacket {
    pub posting_id: i64,
    pub company: String,
    pub title: String,
    pub outcome: PrepOutcome,
    /// The prepared or reused packet; `null` while queued or when it failed.
    #[schema(required = true)]
    pub packet_id: Option<i64>,
    /// The packet's status now; `null` without a packet or when it was deleted.
    #[schema(required = true)]
    pub status: Option<PacketStatus>,
    /// The `prep_packet` run that made (or is making) the packet; `null` when none ran.
    #[schema(required = true)]
    pub run_id: Option<i64>,
    /// Why no packet came out; `null` unless `failed`.
    #[schema(required = true)]
    pub error: Option<String>,
}

/// One plan version.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Plan {
    pub id: i64,
    pub goal_id: i64,
    /// 1, 2, …
    pub version: i32,
    pub author: PlanAuthor,
    pub trigger: PlanTrigger,
    pub status: PlanStatus,
    pub summary: String,
    /// The planner's confidence; `null` for user edits.
    #[schema(required = true)]
    pub confidence: Option<f64>,
    pub companies: Vec<PlanCompany>,
    pub title_keywords: Vec<String>,
    pub locations: Vec<String>,
    pub remote_only: bool,
    pub max_to_score: i32,
    pub shortlist_size: i32,
    pub min_score: i32,
    pub prep_notes: String,
    pub dropped: PlanDropped,
    /// The `plan_goal` run that wrote it; `null` for user edits.
    #[schema(required = true)]
    pub run_id: Option<i64>,
    #[schema(required = true)]
    pub review_item_id: Option<i64>,
    #[schema(required = true)]
    pub claude_call_id: Option<i64>,
    /// The planner call's reported cost in US dollars; 0 for user edits.
    pub cost_usd: f64,
    /// For a `step_failed` version, the step whose failure asked for it.
    #[schema(required = true)]
    pub failed_step: Option<PlanStepKind>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The five steps in order.
    pub steps: Vec<PlanStep>,
    /// The shortlist step's postings, by rank.
    pub shortlist: Vec<ShortlistEntry>,
    /// The prep step's postings in shortlist order, from its output; empty until it ran.
    pub packets: Vec<PlanPacket>,
}

/// One version in a goal's history.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct PlanVersion {
    pub id: i64,
    pub version: i32,
    pub author: PlanAuthor,
    pub trigger: PlanTrigger,
    pub status: PlanStatus,
    pub summary: String,
    pub cost_usd: f64,
    pub created_at: DateTime<Utc>,
}

/// A goal's latest `plan_goal` run.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct GoalRun {
    pub id: i64,
    pub status: RunStatus,
    #[schema(required = true)]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// The `GET /goals/{id}` body.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GoalDetail {
    #[serde(flatten)]
    pub goal: Goal,
    /// Every clarifying question, by ordinal.
    pub questions: Vec<GoalQuestion>,
    /// The current plan with its steps and shortlist; `null` before the first plan.
    #[schema(required = true)]
    pub plan: Option<Plan>,
    /// Every version, newest first.
    pub versions: Vec<PlanVersion>,
    /// The latest `plan_goal` run.
    #[schema(required = true)]
    pub run: Option<GoalRun>,
}

/// The `POST /goals` body.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewGoal {
    /// 1–1000 characters after trimming. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub text: Option<String>,
}

/// One answer in `POST /goals/{id}/answers`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GoalAnswer {
    /// One of the goal's open questions.
    pub question_id: i64,
    /// Trimmed, at most 1000 characters; blank or omitted means skipped.
    #[serde(default)]
    pub answer: Option<String>,
}

/// The `POST /goals/{id}/answers` body.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GoalAnswers {
    #[serde(default)]
    pub answers: Vec<GoalAnswer>,
}

/// One company in `PUT /goals/{id}/plan`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanCompanyInput {
    /// 1–100 characters. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub name: Option<String>,
    /// `greenhouse`, `lever` or `ashby`. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub source: Option<String>,
    /// 1–100 letters, digits, hyphens or underscores, trimmed. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub board: Option<String>,
    /// At most 300 characters are kept.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The `PUT /goals/{id}/plan` body: the whole plan, replacing the current one.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanEdit {
    /// The current plan's id, as the editor loaded it. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub base_plan_id: Option<i64>,
    #[serde(default)]
    pub companies: Vec<PlanCompanyInput>,
    #[serde(default)]
    pub title_keywords: Vec<String>,
    #[serde(default)]
    pub locations: Vec<String>,
    #[serde(default)]
    pub remote_only: bool,
    /// 1–50. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub max_to_score: Option<i64>,
    /// 1–20. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub shortlist_size: Option<i64>,
    /// 0–100. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub min_score: Option<i64>,
    /// At most 1000 characters.
    #[serde(default)]
    pub prep_notes: Option<String>,
}

/// The `POST /goals/{id}/run` body: `{}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunGoal {}

/// The `POST /goals/{id}/replan` body: `{}`, or `{"instruction": "…"}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplanGoal {
    /// What to change; trimmed, at most 1000 characters, blank means none.
    #[serde(default)]
    pub instruction: Option<String>,
}

/// The 409 body of a goal route.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GoalConflict {
    /// Why nothing changed.
    pub message: String,
}

/// Why a goal route failed.
#[derive(Debug)]
pub enum GoalError {
    Api(ApiError),
    /// 409 with a message; nothing changed.
    Conflict(String),
}

impl From<ApiError> for GoalError {
    fn from(err: ApiError) -> Self {
        Self::Api(err)
    }
}

impl<'r> Responder<'r, 'static> for GoalError {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        match self {
            Self::Api(err) => err.respond_to(req),
            Self::Conflict(message) => {
                (Status::Conflict, Json(GoalConflict { message })).respond_to(req)
            }
        }
    }
}

/// Refuses a `text/plain` body with 403. An HTML form can send one (`enctype="text/plain"`)
/// without a CORS preflight, so on these routes it counts as a form, like the bodies
/// [`NotCrossSite`] refuses. Taken after it and before [`JsonContent`].
pub struct NotPlainText;

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for NotPlainText {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> rocket::request::Outcome<Self, ()> {
        let plain = req
            .content_type()
            .is_some_and(|ct| ct.top() == "text" && ct.sub() == "plain");
        if plain {
            rocket::request::Outcome::Error((Status::Forbidden, ()))
        } else {
            rocket::request::Outcome::Success(Self)
        }
    }
}

fn conflict(message: impl Into<String>) -> GoalError {
    GoalError::Conflict(message.into())
}

fn field(field: impl Into<String>, message: impl Into<String>) -> FieldError {
    FieldError {
        field: field.into(),
        message: message.into(),
    }
}

/// The first `max` characters of `text`.
pub(crate) fn head_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// One company before checks, from the planner or the editor.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CompanyDraft<'a> {
    pub name: Option<&'a str>,
    pub source: Option<&'a str>,
    pub board: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Checks one company: a 1–100 character name, a known source and a board slug (both
/// trimmed). A reason is trimmed and cut to [`MAX_REASON`]; blank means none. `Err` names
/// each broken rule as `(field, message)`.
pub(crate) fn check_company(
    draft: CompanyDraft<'_>,
) -> Result<PlanCompanyConfig, Vec<(&'static str, &'static str)>> {
    let mut errors = Vec::new();
    let name = draft.name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        errors.push(("name", REQUIRED));
    } else if name.chars().count() > MAX_NAME {
        errors.push(("name", "At most 100 characters"));
    }
    let source = match draft.source.map(str::trim).unwrap_or_default() {
        "" => {
            errors.push(("source", REQUIRED));
            None
        }
        raw => {
            let source = Source::parse(raw);
            if source.is_none() {
                errors.push(("source", BAD_SOURCE));
            }
            source
        }
    };
    let board = draft.board.map(str::trim).unwrap_or_default();
    if board.is_empty() {
        errors.push(("board", REQUIRED));
    } else if !is_board_slug(board) {
        errors.push(("board", BAD_BOARD));
    }
    let reason = draft
        .reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(|reason| head_chars(reason, MAX_REASON));
    match source {
        Some(source) if errors.is_empty() => Ok(PlanCompanyConfig {
            name: name.to_owned(),
            source,
            board: board.to_owned(),
            reason,
        }),
        _ => Err(errors),
    }
}

/// What [`check_texts`] made of a list of short texts.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CheckedTexts {
    /// The kept items, trimmed, in order.
    pub kept: Vec<String>,
    /// `(index, message)` per item that broke a rule.
    pub problems: Vec<(usize, String)>,
    /// Indexes of valid items past the limit.
    pub overflow: Vec<usize>,
}

/// Checks a list of short texts: each trimmed, 1–`max_len` characters and unique
/// ignoring case; at most `max_items` are kept.
pub(crate) fn check_texts(
    items: &[String],
    max_len: usize,
    max_items: usize,
    duplicate: &str,
) -> CheckedTexts {
    let mut checked = CheckedTexts::default();
    let mut seen = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let item = item.trim();
        if item.is_empty() {
            checked.problems.push((index, REQUIRED.into()));
        } else if item.chars().count() > max_len {
            checked
                .problems
                .push((index, format!("At most {max_len} characters")));
        } else if !seen.insert(item.to_lowercase()) {
            checked.problems.push((index, duplicate.into()));
        } else if checked.kept.len() == max_items {
            checked.overflow.push(index);
        } else {
            checked.kept.push(item.to_owned());
        }
    }
    checked
}

/// An integer in `range`, or `None`.
fn in_range(value: i64, range: std::ops::RangeInclusive<i64>) -> Option<i32> {
    range
        .contains(&value)
        .then(|| i32::try_from(value).ok())
        .flatten()
}

/// Checks a `PUT /goals/{id}/plan` body against the plan limits (see docs/goals.md),
/// naming every failing field.
pub fn validate_config(edit: &PlanEdit) -> Result<PlanConfig, Vec<FieldError>> {
    let mut errors = Vec::new();
    if edit.companies.is_empty() {
        errors.push(field("companies", REQUIRED));
    } else if edit.companies.len() > MAX_COMPANIES {
        errors.push(field("companies", TOO_MANY_COMPANIES));
    }
    let mut companies = Vec::new();
    let mut boards = HashSet::new();
    for (index, input) in edit.companies.iter().enumerate() {
        match check_company(CompanyDraft {
            name: input.name.as_deref(),
            source: input.source.as_deref(),
            board: input.board.as_deref(),
            reason: input.reason.as_deref(),
        }) {
            Ok(company) => {
                if boards.insert((company.source, company.board.clone())) {
                    companies.push(company);
                } else {
                    errors.push(field(format!("companies[{index}].board"), DUPLICATE_BOARD));
                }
            }
            Err(problems) => {
                for (name, message) in problems {
                    errors.push(field(format!("companies[{index}].{name}"), message));
                }
            }
        }
    }

    let mut texts = |name: &str, items: &[String], max_len, max_items, what: &str| {
        let checked = check_texts(items, max_len, max_items, &format!("Duplicate {what}"));
        for (index, message) in &checked.problems {
            errors.push(field(format!("{name}[{index}]"), message.clone()));
        }
        if !checked.overflow.is_empty() {
            errors.push(field(name, format!("At most {max_items} {what}s")));
        }
        checked.kept
    };
    let title_keywords = texts(
        "title_keywords",
        &edit.title_keywords,
        MAX_KEYWORD,
        MAX_KEYWORDS,
        "keyword",
    );
    let locations = texts(
        "locations",
        &edit.locations,
        MAX_LOCATION,
        MAX_LOCATIONS,
        "location",
    );
    if edit.title_keywords.is_empty() {
        errors.push(field("title_keywords", REQUIRED));
    }

    let mut number = |name: &str, value: Option<i64>, range, message: &str| match value {
        None => {
            errors.push(field(name, REQUIRED));
            None
        }
        Some(value) => {
            let checked = in_range(value, range);
            if checked.is_none() {
                errors.push(field(name, message));
            }
            checked
        }
    };
    let max_to_score = number(
        "max_to_score",
        edit.max_to_score,
        1..=MAX_TO_SCORE,
        BAD_MAX_TO_SCORE,
    );
    let shortlist_size = number(
        "shortlist_size",
        edit.shortlist_size,
        1..=MAX_SHORTLIST,
        BAD_SHORTLIST_SIZE,
    );
    let min_score = number("min_score", edit.min_score, 0..=100, BAD_MIN_SCORE);
    let prep_notes = text(
        &mut errors,
        "prep_notes",
        edit.prep_notes.as_deref(),
        MAX_PREP_NOTES,
    )
    .unwrap_or_default();

    match (max_to_score, shortlist_size, min_score) {
        (Some(max_to_score), Some(shortlist_size), Some(min_score)) if errors.is_empty() => {
            Ok(PlanConfig {
                companies,
                title_keywords,
                locations,
                remote_only: edit.remote_only,
                max_to_score,
                shortlist_size,
                min_score,
                prep_notes,
            })
        }
        _ => Err(errors),
    }
}

/// Locks goal `id` (`FOR UPDATE`) inside `tx`.
pub(crate) async fn lock_goal(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
) -> sqlx::Result<Option<Goal>> {
    sqlx::query_as(&format!(
        "SELECT {GOAL_COLUMNS} FROM goals WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
}

/// Sets goal `id`'s status and error inside `tx`.
pub(crate) async fn set_status(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
    status: GoalStatus,
    error: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE goals SET status = $2, error = $3, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status.as_str())
        .bind(error)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Queues a `plan_goal` run with `input` inside `tx`. The caller commits, publishes the
/// run and wakes the worker.
pub(crate) async fn queue_planner(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    input: Value,
) -> sqlx::Result<runs::Run> {
    runs::insert_run(
        tx,
        bus,
        plan_goal::WORKFLOW,
        &[plan_goal::STEP],
        input,
        Duration::ZERO,
    )
    .await
}

/// Publishes `goal.updated` with goal `id`'s current row. Best effort: a read error is
/// logged, and an unknown id publishes nothing.
pub async fn publish(pool: &PgPool, bus: &EventBus, id: i64) {
    match store::goal(pool, id).await {
        Ok(Some(goal)) => bus.publish(Event::GoalUpdated { goal }),
        Ok(None) => {}
        Err(err) => rocket::warn!("goals: cannot publish goal {id}: {err}"),
    }
}

/// Reads goal `id`'s detail after a committed write, publishes `goal.updated` with it
/// and returns it.
async fn committed(pool: &PgPool, bus: &EventBus, id: i64) -> Result<GoalDetail, GoalError> {
    let detail = store::detail(pool, id)
        .await
        .map_err(internal("read a goal"))?
        .ok_or(ApiError::NotFound)?;
    bus.publish(Event::GoalUpdated {
        goal: detail.goal.clone(),
    });
    Ok(detail)
}

/// `GET /goals`: goals newest first.
#[utoipa::path(
    get,
    path = "/goals",
    tag = "goals",
    operation_id = "list_goals",
    responses(
        (status = 200, description = "Up to 200 goals, `id` descending", body = [Goal]),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The goals could not be read"),
    )
)]
#[rocket::get("/goals")]
pub async fn list(_host: LocalHost, pool: &State<PgPool>) -> Result<Json<Vec<Goal>>, ApiError> {
    store::list(pool)
        .await
        .map(Json)
        .map_err(internal("list goals"))
}

/// `GET /goals/{id}`: a goal with its questions, current plan, versions and latest run.
#[utoipa::path(
    get,
    path = "/goals/{id}",
    tag = "goals",
    operation_id = "get_goal",
    params(("id" = i64, Path, description = "The goal's id")),
    responses(
        (status = 200, description = "The goal, its questions by ordinal, its current plan with steps, shortlist and prep packets (with their live status), its versions newest first and its latest `plan_goal` run", body = GoalDetail),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 404, description = "No goal has this id"),
        (status = 500, description = "The goal could not be read"),
    )
)]
#[rocket::get("/goals/<id>")]
pub async fn show(
    _host: LocalHost,
    id: i64,
    pool: &State<PgPool>,
) -> Result<Json<GoalDetail>, ApiError> {
    store::detail(pool, id)
        .await
        .map_err(internal("read a goal"))?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// `POST /goals`: stores a goal and queues its planner.
#[utoipa::path(
    post,
    path = "/goals",
    tag = "goals",
    operation_id = "create_goal",
    request_body = NewGoal,
    responses(
        (status = 201, description = "The goal, `planning`, with its queued `plan_goal` run", body = GoalDetail),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form body (`application/x-www-form-urlencoded`, `multipart/form-data` or `text/plain`)"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`text` is blank or longer than 1000 characters", body = ValidationErrors),
        (status = 500, description = "The goal could not be stored"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/goals", data = "<body>")]
pub async fn create(
    _host: LocalHost,
    _site: NotCrossSite,
    _plain: NotPlainText,
    _json: JsonContent,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<NewGoal>, json::Error<'_>>,
) -> Result<(Status, Json<GoalDetail>), GoalError> {
    let NewGoal { text: goal_text } = parsed(body)?;
    let mut errors = Vec::new();
    let goal_text = text(&mut errors, "text", goal_text.as_deref(), MAX_GOAL);
    if goal_text.is_none() {
        errors.push(field("text", REQUIRED));
    }
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }).into());
    }
    let failed = |err: sqlx::Error| internal("create a goal")(err);
    let mut tx = pool.begin().await.map_err(failed)?;
    let id: i64 =
        sqlx::query_scalar("INSERT INTO goals (text, status) VALUES ($1, 'planning') RETURNING id")
            .bind(goal_text)
            .fetch_one(&mut *tx)
            .await
            .map_err(failed)?;
    let run = queue_planner(&mut tx, bus, json!({"goal_id": id, "trigger": "goal"}))
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    bus.publish(Event::RunUpdated { run });
    wake.0.notify_one();
    Ok((Status::Created, Json(committed(pool, bus, id).await?)))
}

/// `POST /goals/{id}/answers`: answers the open clarifying questions and plans again.
#[utoipa::path(
    post,
    path = "/goals/{id}/answers",
    tag = "goals",
    operation_id = "answer_goal_questions",
    params(("id" = i64, Path, description = "The goal's id")),
    request_body = GoalAnswers,
    responses(
        (status = 202, description = "The answers are stored, every open question is answered (blank ones skipped) and a `plan_goal` run is queued; the goal is `planning`", body = GoalDetail),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form body (`application/x-www-form-urlencoded`, `multipart/form-data` or `text/plain`)"),
        (status = 404, description = "No goal has this id"),
        (status = 409, description = "The goal isn't waiting for answers; nothing changed", body = GoalConflict),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "An answer is longer than 1000 characters, or names a question that isn't one of the goal's open ones", body = ValidationErrors),
        (status = 500, description = "The answers could not be stored"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/goals/<id>/answers", data = "<body>")]
pub async fn answer(
    _host: LocalHost,
    _site: NotCrossSite,
    _plain: NotPlainText,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<GoalAnswers>, json::Error<'_>>,
) -> Result<(Status, Json<GoalDetail>), GoalError> {
    let GoalAnswers { answers } = parsed(body)?;
    let mut errors = Vec::new();
    let answers: Vec<(i64, Option<String>)> = answers
        .iter()
        .enumerate()
        .map(|(index, given)| {
            let answer = text(
                &mut errors,
                &format!("answers[{index}].answer"),
                given.answer.as_deref(),
                MAX_ANSWER,
            );
            (given.question_id, answer)
        })
        .collect();
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }).into());
    }

    let failed = |err: sqlx::Error| internal("answer a goal's questions")(err);
    let mut tx = pool.begin().await.map_err(failed)?;
    let goal = lock_goal(&mut tx, id)
        .await
        .map_err(failed)?
        .ok_or(ApiError::NotFound)?;
    if goal.status != GoalStatus::NeedsAnswers {
        return Err(conflict("This goal isn't waiting for answers"));
    }
    let open: HashSet<i64> = sqlx::query_scalar(
        "SELECT id FROM goal_questions WHERE goal_id = $1 AND answered_at IS NULL",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(failed)?
    .into_iter()
    .collect();
    let mut answered = HashSet::new();
    for (index, (question_id, _)) in answers.iter().enumerate() {
        if !open.contains(question_id) {
            errors.push(field(
                format!("answers[{index}].question_id"),
                NOT_OPEN_QUESTION,
            ));
        } else if !answered.insert(*question_id) {
            errors.push(field(
                format!("answers[{index}].question_id"),
                DUPLICATE_QUESTION,
            ));
        }
    }
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }).into());
    }
    for (question_id, answer) in &answers {
        sqlx::query("UPDATE goal_questions SET answer = $2 WHERE id = $1")
            .bind(question_id)
            .bind(answer)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
    }
    sqlx::query(
        "UPDATE goal_questions SET answered_at = now() WHERE goal_id = $1 AND answered_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    set_status(&mut tx, id, GoalStatus::Planning, None)
        .await
        .map_err(failed)?;
    let run = queue_planner(&mut tx, bus, json!({"goal_id": id, "trigger": "goal"}))
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    bus.publish(Event::RunUpdated { run });
    wake.0.notify_one();
    Ok((Status::Accepted, Json(committed(pool, bus, id).await?)))
}

/// `PUT /goals/{id}/plan`: replaces the plan with the user's own, as a new version.
#[utoipa::path(
    put,
    path = "/goals/{id}/plan",
    tag = "goals",
    operation_id = "edit_goal_plan",
    params(("id" = i64, Path, description = "The goal's id")),
    request_body = PlanEdit,
    responses(
        (status = 200, description = "The goal with its new plan version (`author: user`, `trigger: user_edit`, `ready`), finished steps kept where their inputs are unchanged; or unchanged when the body equals the current plan. A `running` goal goes on from the first changed step; a `completed` or `failed` one becomes `planned`", body = GoalDetail),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form body (`application/x-www-form-urlencoded`, `multipart/form-data` or `text/plain`)"),
        (status = 404, description = "No goal has this id"),
        (status = 409, description = "The goal is `planning` or `needs_answers`, has no plan, or `base_plan_id` isn't its current plan; nothing changed", body = GoalConflict),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "A field breaks the plan limits; every failing field is named, e.g. `companies[1].board`", body = ValidationErrors),
        (status = 500, description = "The plan could not be stored"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::put("/goals/<id>/plan", data = "<body>")]
pub async fn edit_plan(
    _host: LocalHost,
    _site: NotCrossSite,
    _plain: NotPlainText,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<PlanEdit>, json::Error<'_>>,
) -> Result<Json<GoalDetail>, GoalError> {
    let edit = parsed(body)?;
    let failed = |err: sqlx::Error| internal("edit a goal's plan")(err);
    let mut tx = pool.begin().await.map_err(failed)?;
    let goal = lock_goal(&mut tx, id)
        .await
        .map_err(failed)?
        .ok_or(ApiError::NotFound)?;
    if matches!(goal.status, GoalStatus::Planning | GoalStatus::NeedsAnswers) {
        return Err(conflict(
            "The plan can't be edited while Waypoint is planning",
        ));
    }
    let config = match (validate_config(&edit), edit.base_plan_id) {
        (Ok(config), Some(_)) => config,
        (checked, base) => {
            let mut errors = checked.err().unwrap_or_default();
            if base.is_none() {
                errors.insert(0, field("base_plan_id", REQUIRED));
            }
            return Err(ApiError::Invalid(ValidationErrors { errors }).into());
        }
    };
    let Some(current) = goal.current_plan_id else {
        return Err(conflict("This goal has no plan to edit"));
    };
    if edit.base_plan_id != Some(current) {
        return Err(conflict("This plan changed since you opened it"));
    }
    let base = store::plan_row(&mut *tx, current)
        .await
        .map_err(failed)?
        .ok_or(ApiError::NotFound)?;
    if base.config.0 == config {
        tx.commit().await.map_err(failed)?;
        let detail = store::detail(pool, id)
            .await
            .map_err(failed)?
            .ok_or(ApiError::NotFound)?;
        return Ok(Json(detail));
    }
    store::write_version(
        &mut tx,
        id,
        &store::PlanDraft {
            author: PlanAuthor::User,
            trigger: PlanTrigger::UserEdit,
            status: PlanStatus::Ready,
            summary: base.summary,
            confidence: None,
            config,
            dropped: PlanDropped::default(),
            run_id: None,
            review_item_id: None,
            claude_call_id: None,
            cost_usd: 0.0,
        },
    )
    .await
    .map_err(failed)?;
    let tick = match goal.status {
        GoalStatus::Running => {
            exec::request_tick_tx(&mut tx, bus, id)
                .await
                .map_err(failed)?;
            true
        }
        GoalStatus::Completed | GoalStatus::Failed => {
            set_status(&mut tx, id, GoalStatus::Planned, None)
                .await
                .map_err(failed)?;
            false
        }
        _ => false,
    };
    tx.commit().await.map_err(failed)?;
    if tick {
        wake.0.notify_one();
    }
    Ok(Json(committed(pool, bus, id).await?))
}

/// `POST /goals/{id}/run`: runs the current plan from its first unfinished step.
#[utoipa::path(
    post,
    path = "/goals/{id}/run",
    tag = "goals",
    operation_id = "run_goal",
    params(("id" = i64, Path, description = "The goal's id")),
    request_body = RunGoal,
    responses(
        (status = 202, description = "The goal is `running`: failed steps are `pending` again, the automatic replan count is 0 and a `plan_step` tick is queued", body = GoalDetail),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form body (`application/x-www-form-urlencoded`, `multipart/form-data` or `text/plain`)"),
        (status = 404, description = "No goal has this id"),
        (status = 409, description = "The goal isn't `planned` or `failed`, or its current plan isn't `ready`; nothing changed", body = GoalConflict),
        (status = 415, description = "The body is not `application/json`"),
        (status = 500, description = "The run could not be started"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/goals/<id>/run", data = "<body>")]
pub async fn run(
    _host: LocalHost,
    _site: NotCrossSite,
    _plain: NotPlainText,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<RunGoal>, json::Error<'_>>,
) -> Result<(Status, Json<GoalDetail>), GoalError> {
    let RunGoal {} = parsed(body)?;
    let failed = |err: sqlx::Error| internal("run a goal")(err);
    let mut tx = pool.begin().await.map_err(failed)?;
    let goal = lock_goal(&mut tx, id)
        .await
        .map_err(failed)?
        .ok_or(ApiError::NotFound)?;
    if !matches!(goal.status, GoalStatus::Planned | GoalStatus::Failed) {
        return Err(conflict(match goal.status {
            GoalStatus::Running => "This plan is already running",
            GoalStatus::Completed => "This plan already ran; edit it or replan to run again",
            _ => "This goal has no plan to run yet",
        }));
    }
    let Some(plan_id) = goal.current_plan_id else {
        return Err(conflict("This goal has no plan to run yet"));
    };
    let status: PlanStatus = sqlx::query_scalar("SELECT status FROM plans WHERE id = $1")
        .bind(plan_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(failed)?;
    match status {
        PlanStatus::Ready => {}
        PlanStatus::InReview => return Err(conflict("This plan needs review before it can run")),
        PlanStatus::Dismissed => {
            return Err(conflict("You dismissed this plan. Edit it or replan"));
        }
    }
    sqlx::query(
        "DELETE FROM plan_step_tasks WHERE step_id IN \
         (SELECT id FROM plan_steps WHERE plan_id = $1 AND status = 'failed')",
    )
    .bind(plan_id)
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    sqlx::query(
        "UPDATE plan_steps SET status = 'pending', output = NULL, last_error = NULL, \
         started_at = NULL, finished_at = NULL, updated_at = now() \
         WHERE plan_id = $1 AND status = 'failed'",
    )
    .bind(plan_id)
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    sqlx::query(
        "UPDATE goals SET status = 'running', error = NULL, auto_replans = 0, \
         updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    exec::request_tick_tx(&mut tx, bus, id)
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    wake.0.notify_one();
    Ok((Status::Accepted, Json(committed(pool, bus, id).await?)))
}

/// `POST /goals/{id}/replan`: asks the planner for a new version.
#[utoipa::path(
    post,
    path = "/goals/{id}/replan",
    tag = "goals",
    operation_id = "replan_goal",
    params(("id" = i64, Path, description = "The goal's id")),
    request_body = ReplanGoal,
    responses(
        (status = 202, description = "A `plan_goal` run is queued (`user_replan` with the current plan, or `goal` when there is none) and the goal is `planning`", body = GoalDetail),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form body (`application/x-www-form-urlencoded`, `multipart/form-data` or `text/plain`)"),
        (status = 404, description = "No goal has this id"),
        (status = 409, description = "The goal isn't `planned`, `completed` or `failed`; nothing changed", body = GoalConflict),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "`instruction` is longer than 1000 characters", body = ValidationErrors),
        (status = 500, description = "The replan could not be queued"),
    )
)]
#[allow(clippy::too_many_arguments)]
#[rocket::post("/goals/<id>/replan", data = "<body>")]
pub async fn replan(
    _host: LocalHost,
    _site: NotCrossSite,
    _plain: NotPlainText,
    _json: JsonContent,
    id: i64,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<ReplanGoal>, json::Error<'_>>,
) -> Result<(Status, Json<GoalDetail>), GoalError> {
    let ReplanGoal { instruction } = parsed(body)?;
    let mut errors = Vec::new();
    let instruction = text(
        &mut errors,
        "instruction",
        instruction.as_deref(),
        MAX_INSTRUCTION,
    );
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }).into());
    }
    let failed = |err: sqlx::Error| internal("replan a goal")(err);
    let mut tx = pool.begin().await.map_err(failed)?;
    let goal = lock_goal(&mut tx, id)
        .await
        .map_err(failed)?
        .ok_or(ApiError::NotFound)?;
    if !matches!(
        goal.status,
        GoalStatus::Planned | GoalStatus::Completed | GoalStatus::Failed
    ) {
        return Err(conflict(match goal.status {
            GoalStatus::Running => "This plan is running; wait for it to finish",
            _ => "Waypoint is already planning this goal",
        }));
    }
    let input = match goal.current_plan_id {
        Some(plan_id) => json!({
            "goal_id": id, "trigger": "user_replan", "plan_id": plan_id,
            "instruction": instruction,
        }),
        None => json!({"goal_id": id, "trigger": "goal"}),
    };
    set_status(&mut tx, id, GoalStatus::Planning, None)
        .await
        .map_err(failed)?;
    let run = queue_planner(&mut tx, bus, input).await.map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    bus.publish(Event::RunUpdated { run });
    wake.0.notify_one();
    Ok((Status::Accepted, Json(committed(pool, bus, id).await?)))
}
