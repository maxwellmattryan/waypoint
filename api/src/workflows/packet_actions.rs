//! Acting on a prepared application packet (see docs/application-prep.md): approving it,
//! possibly with edits, through the approval gate, and marking it applied once the user has
//! submitted it themselves. Nothing here contacts a job board or sends anything.
//!
//! - **Approval.** A `ready` packet gets one `approve_packet` run ([`request_approval`]),
//!   whose one step, `approve`, calls the gated `write` tool
//!   [`ApproveApplicationPacket`] ([`Tool::NEEDS_APPROVAL`]) with the packet's draft. The
//!   run parks with one `pending` action. Approving it (`runs::approvals::approve`, after
//!   [`check_approval`]) runs the handler, which stores the approved resume and answers and
//!   makes the packet `approved`; rejecting it (`runs::approvals::reject`, through
//!   [`mark_rejected`]) makes the packet `dismissed`. Undo returns an `approved` packet to
//!   `ready`.
//! - **Mark applied.** An `approved` packet gets an `apply_packet` run on request
//!   ([`queue_apply`]), whose one step, `apply`, calls the `write` tool
//!   [`MarkPacketApplied`]. It creates the posting's application, or updates the one an
//!   earlier packet of the posting was applied with, and makes the packet `applied`. Undo
//!   deletes a created application or restores an updated one, and returns the packet to
//!   `approved`.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use super::prep_packet::{
    MAX_ANSWER, PacketAnswer, PacketQuestion, PacketQueued, PacketStatus, QUESTIONS,
};
use super::score_fit::{FitRun, head_chars};
use crate::applications::timeline::{self, EventKind};
use crate::applications::{self, Application, ApplicationStatus, MAX_SHORT, MAX_URL, is_web_url};
use crate::events::{Event, EventBus};
use crate::runs::approvals::{DecisionError, PendingAction};
use crate::runs::{self, RUN_COLUMNS, Run, StepContext, StepError, Workflow};
use crate::tools::{Permission, Tool, ToolContext, ToolError, UndoError};

/// The approval workflow's name.
pub const APPROVE_WORKFLOW: &str = "approve_packet";
/// Its one step.
pub const APPROVE_STEP: &str = "approve";
/// The gated `write` tool that approves a packet.
pub const APPROVE_TOOL: &str = "approve_application_packet";
/// The mark-applied workflow's name.
pub const APPLY_WORKFLOW: &str = "apply_packet";
/// Its one step.
pub const APPLY_STEP: &str = "apply";
/// The `write` tool that marks a packet applied.
pub const APPLY_TOOL: &str = "mark_packet_applied";

/// The error a run gets when its packet's save is undone while the approval waits.
pub const UNDONE: &str = "the packet was undone";

/// The `POST /packets/{id}/approval` body: `{}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestApprovalRequest {}

/// The `POST /packets/{id}/applied` body: `{}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkAppliedRequest {}

/// Why [`request_approval`] or [`queue_apply`] queued nothing.
#[derive(Debug)]
pub enum PacketActionError {
    /// No packet has the id.
    NotFound,
    /// The packet isn't `ready`, so it can't be approved.
    NotReady,
    /// The packet isn't `approved`, or its approved resume has been deleted.
    NotApproved,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for PacketActionError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}

/// The input of both workflows.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunInput {
    #[allow(dead_code)]
    posting_id: i64,
    packet_id: i64,
}

/// Locks packet `packet_id` inside `tx` and returns `(posting_id, status,
/// approved_resume_version_id)`.
async fn lock_packet(
    tx: &mut Transaction<'_, Postgres>,
    packet_id: i64,
) -> Result<(i64, PacketStatus, Option<i64>), PacketActionError> {
    sqlx::query_as(
        "SELECT posting_id, status, approved_resume_version_id FROM application_packets \
         WHERE id = $1 FOR UPDATE",
    )
    .bind(packet_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(PacketActionError::NotFound)
}

/// The latest `workflow` run for `packet_id` in one of `statuses`, if any.
async fn active_run(
    tx: &mut Transaction<'_, Postgres>,
    workflow: &str,
    packet_id: i64,
    statuses: &[&str],
) -> sqlx::Result<Option<Run>> {
    sqlx::query_as(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE workflow = $1 \
         AND input -> 'packet_id' = to_jsonb($2::bigint) AND status = ANY($3) \
         ORDER BY id DESC LIMIT 1"
    ))
    .bind(workflow)
    .bind(packet_id)
    .bind(statuses)
    .fetch_optional(&mut **tx)
    .await
}

/// Queues the `approve_packet` run for `ready` packet `packet_id` inside `tx`, or returns
/// the one already waiting, running or awaiting approval (`reused: true`). The caller
/// commits, then, when the run isn't reused, publishes `run.updated` and wakes the worker.
pub async fn request_approval(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    packet_id: i64,
) -> Result<PacketQueued, PacketActionError> {
    let (posting_id, status, _) = lock_packet(tx, packet_id).await?;
    if status != PacketStatus::Ready {
        return Err(PacketActionError::NotReady);
    }
    let waiting = ["queued", "paused", "running", "awaiting_approval"];
    if let Some(run) = active_run(tx, APPROVE_WORKFLOW, packet_id, &waiting).await? {
        return Ok(PacketQueued { run, reused: true });
    }
    let run = runs::insert_run(
        tx,
        bus,
        APPROVE_WORKFLOW,
        &[APPROVE_STEP],
        json!({"posting_id": posting_id, "packet_id": packet_id}),
        Duration::ZERO,
    )
    .await?;
    Ok(PacketQueued { run, reused: false })
}

/// Queues an `apply_packet` run for `approved` packet `packet_id` inside `tx`, or returns
/// the one already waiting or running (`reused: true`). The packet's approved resume must
/// still exist. The caller commits, publishes and wakes as for [`request_approval`].
pub async fn queue_apply(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    packet_id: i64,
) -> Result<PacketQueued, PacketActionError> {
    let (posting_id, status, resume) = lock_packet(tx, packet_id).await?;
    if status != PacketStatus::Approved || resume.is_none() {
        return Err(PacketActionError::NotApproved);
    }
    let waiting = ["queued", "paused", "running"];
    if let Some(run) = active_run(tx, APPLY_WORKFLOW, packet_id, &waiting).await? {
        return Ok(PacketQueued { run, reused: true });
    }
    let run = runs::insert_run(
        tx,
        bus,
        APPLY_WORKFLOW,
        &[APPLY_STEP],
        json!({"posting_id": posting_id, "packet_id": packet_id}),
        Duration::ZERO,
    )
    .await?;
    Ok(PacketQueued { run, reused: false })
}

/// The latest `approve_packet` or `apply_packet` run of each of `packet_ids`.
pub(crate) async fn latest_runs(
    pool: &PgPool,
    packet_ids: &[i64],
) -> sqlx::Result<HashMap<i64, FitRun>> {
    if packet_ids.is_empty() {
        return Ok(HashMap::new());
    }
    #[derive(sqlx::FromRow)]
    struct Latest {
        packet: Value,
        #[sqlx(flatten)]
        run: FitRun,
    }
    let found: Vec<Latest> = sqlx::query_as(
        "SELECT DISTINCT ON (input -> 'packet_id') input -> 'packet_id' AS packet, \
         id, status, error FROM agent_runs WHERE workflow IN ($1, $2) \
         AND input -> 'packet_id' IN (SELECT to_jsonb(id) FROM unnest($3::bigint[]) AS id) \
         ORDER BY input -> 'packet_id', id DESC",
    )
    .bind(APPROVE_WORKFLOW)
    .bind(APPLY_WORKFLOW)
    .bind(packet_ids)
    .fetch_all(pool)
    .await?;
    Ok(found
        .into_iter()
        .filter_map(|latest| latest.packet.as_i64().map(|id| (id, latest.run)))
        .collect())
}

/// The five answers of an approval request, by question key; `null` for no answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalAnswers {
    pub work_authorization: Option<String>,
    pub notice_period: Option<String>,
    pub salary_expectation: Option<String>,
    pub why_leaving: Option<String>,
    pub why_company: Option<String>,
}

impl ApprovalAnswers {
    /// The answer to `key`, as given.
    pub fn get(&self, key: PacketQuestion) -> Option<&str> {
        match key {
            PacketQuestion::WorkAuthorization => self.work_authorization.as_deref(),
            PacketQuestion::NoticePeriod => self.notice_period.as_deref(),
            PacketQuestion::SalaryExpectation => self.salary_expectation.as_deref(),
            PacketQuestion::WhyLeaving => self.why_leaving.as_deref(),
            PacketQuestion::WhyCompany => self.why_company.as_deref(),
        }
    }

    /// The draft's answers.
    fn of(answers: &[PacketAnswer]) -> Self {
        let get = |key: PacketQuestion| {
            answers
                .iter()
                .find(|answer| answer.key == key)
                .and_then(|answer| answer.answer.clone())
        };
        Self {
            work_authorization: get(PacketQuestion::WorkAuthorization),
            notice_period: get(PacketQuestion::NoticePeriod),
            salary_expectation: get(PacketQuestion::SalaryExpectation),
            why_leaving: get(PacketQuestion::WhyLeaving),
            why_company: get(PacketQuestion::WhyCompany),
        }
    }
}

/// The [`ApproveApplicationPacket`] input, and what its pending action asks to approve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalInput {
    pub packet_id: i64,
    /// The resume to send; `null` when the draft's resume has been deleted.
    pub resume_version_id: Option<i64>,
    pub answers: ApprovalAnswers,
}

/// The approval request for a packet's draft.
pub fn draft_input(
    packet_id: i64,
    resume_version_id: Option<i64>,
    answers: &[PacketAnswer],
) -> Value {
    json!(ApprovalInput {
        packet_id,
        resume_version_id,
        answers: ApprovalAnswers::of(answers),
    })
}

/// The path of the first answer of `answers` longer than [`MAX_ANSWER`] characters after
/// trimming, if any.
fn too_long(answers: &ApprovalAnswers) -> Option<String> {
    QUESTIONS.iter().find_map(|(key, _)| {
        answers
            .get(*key)
            .filter(|text| text.trim().chars().count() > MAX_ANSWER)
            .map(|_| format!("$.answers.{}", key.as_str()))
    })
}

/// The approved answers: all five in question order with the question for `company`, each
/// trimmed and a blank one `null`, keeping a draft answer's citations only when its text
/// didn't change.
pub fn approved_answers(
    draft: &[PacketAnswer],
    approved: &ApprovalAnswers,
    company: &str,
) -> Vec<PacketAnswer> {
    QUESTIONS
        .iter()
        .map(|(key, _)| {
            let answer = approved
                .get(*key)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            let drafted = draft.iter().find(|answer| answer.key == *key);
            let citations = match drafted {
                Some(drafted) if drafted.answer == answer => drafted.citations.clone(),
                _ => Vec::new(),
            };
            PacketAnswer {
                key: *key,
                question: key.text(company),
                answer,
                citations,
            }
        })
        .collect()
}

/// Checks an approval of an [`APPROVE_TOOL`] action before it is stored, inside the
/// decision's transaction; a no-op for any other tool. It locks the packet, and refuses
/// with a conflict unless the action's run is the packet's `approve_packet` run and the
/// packet is `ready`, or as invalid when `approved` changes `packet_id`, names no existing
/// resume or has an answer over [`MAX_ANSWER`] characters. Messages never quote a value.
pub(crate) async fn check_approval(
    tx: &mut Transaction<'_, Postgres>,
    action: &PendingAction,
    approved: &Value,
) -> Result<(), DecisionError> {
    if action.tool != APPROVE_TOOL {
        return Ok(());
    }
    let input = ApprovalInput::deserialize(approved)
        .map_err(|_| DecisionError::Invalid("$: does not fit the tool's input type".into()))?;
    if action.input.get("packet_id") != Some(&json!(input.packet_id)) {
        return Err(DecisionError::Invalid(
            "$.packet_id: can't be changed".into(),
        ));
    }
    let id = input.packet_id;
    let status: Option<PacketStatus> =
        sqlx::query_scalar("SELECT status FROM application_packets WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    let Some(status) = status else {
        return Err(DecisionError::Conflict(format!("packet {id} is gone")));
    };
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM agent_runs WHERE id = $1 AND workflow = $2 \
         AND input -> 'packet_id' = to_jsonb($3::bigint))",
    )
    .bind(action.run_id)
    .bind(APPROVE_WORKFLOW)
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    if !owns {
        return Err(DecisionError::Conflict(format!(
            "run {} is not packet {id}'s approval run",
            action.run_id
        )));
    }
    if status != PacketStatus::Ready {
        return Err(DecisionError::Conflict(format!(
            "packet {id} is {}, not ready",
            status.as_str()
        )));
    }
    let Some(resume) = input.resume_version_id else {
        return Err(DecisionError::Invalid(
            "$.resume_version_id: choose a resume".into(),
        ));
    };
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM resume_versions WHERE id = $1)")
            .bind(resume)
            .fetch_one(&mut **tx)
            .await?;
    if !exists {
        return Err(DecisionError::Invalid(
            "$.resume_version_id: no resume has this id".into(),
        ));
    }
    if let Some(path) = too_long(&input.answers) {
        return Err(DecisionError::Invalid(format!(
            "{path}: at most {MAX_ANSWER} characters"
        )));
    }
    Ok(())
}

/// Rejecting run `run_id`'s pending action inside the decision's transaction: when it is an
/// `approve_packet` run, its `ready` packet becomes `dismissed`.
pub(crate) async fn mark_rejected(conn: &mut PgConnection, run_id: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE application_packets SET status = 'dismissed', updated_at = now() \
         WHERE status = 'ready' AND to_jsonb(id) = \
         (SELECT input -> 'packet_id' FROM agent_runs WHERE id = $1 AND workflow = $2)",
    )
    .bind(run_id)
    .bind(APPROVE_WORKFLOW)
    .execute(conn)
    .await
    .map(|_| ())
}

/// Before packet `packet_id`'s save is undone, inside the undo's transaction: locks its
/// approval actions and refuses while one is `approved` or `executing`. Each `pending`
/// one becomes `rejected`, its parked steps fail and its run ends `rejected` with
/// [`UNDONE`], as a rejection does but without a signal.
pub(crate) async fn reject_pending_approvals(
    conn: &mut PgConnection,
    packet_id: i64,
) -> Result<(), UndoError> {
    let actions: Vec<(i64, String)> = sqlx::query_as(
        "SELECT pa.run_id, pa.status FROM pending_actions pa \
         JOIN agent_runs r ON r.id = pa.run_id \
         WHERE pa.tool = $2 AND r.workflow = $3 AND r.input -> 'packet_id' = to_jsonb($1::bigint) \
         ORDER BY pa.id FOR UPDATE OF pa",
    )
    .bind(packet_id)
    .bind(APPROVE_TOOL)
    .bind(APPROVE_WORKFLOW)
    .fetch_all(&mut *conn)
    .await?;
    if actions
        .iter()
        .any(|(_, status)| status == "approved" || status == "executing")
    {
        return Err(UndoError::Conflict(format!(
            "packet {packet_id}'s approval is being carried out"
        )));
    }
    let pending: Vec<i64> = actions
        .into_iter()
        .filter(|(_, status)| status == "pending")
        .map(|(run, _)| run)
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE pending_actions SET status = 'rejected', decided_at = now() \
         WHERE run_id = ANY($1) AND status = 'pending'",
    )
    .bind(&pending)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE run_steps SET status = 'failed', error = $2, finished_at = now(), \
         updated_at = now() WHERE run_id = ANY($1) AND status = 'awaiting_approval'",
    )
    .bind(&pending)
    .bind(UNDONE)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE agent_runs SET status = 'rejected', error = $2, finished_at = now(), \
         updated_at = now() WHERE id = ANY($1)",
    )
    .bind(&pending)
    .bind(UNDONE)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn db(what: &'static str) -> impl FnOnce(sqlx::Error) -> StepError {
    move |err| StepError::Retry(format!("cannot {what}: {err}"))
}

fn tool_db(what: &'static str) -> impl FnOnce(sqlx::Error) -> ToolError {
    move |err| {
        rocket::warn!("packet_actions: cannot {what}: {err}");
        ToolError::Failed(format!("cannot {what}"))
    }
}

/// The `approve` step: asks to approve the packet's draft. A packet that is gone, or no
/// longer `ready` before anything was asked, isn't asked about (`approved: false`). A
/// rerun asks with the input the step first asked with, so it finds the same pending
/// action.
async fn approve(ctx: &mut StepContext<'_>, input: Value) -> Result<Value, StepError> {
    let input = RunInput::deserialize(&input)
        .map_err(|err| StepError::Fail(format!("invalid input: {err}")))?;
    let id = input.packet_id;
    let pool = ctx.pool().clone();
    let asked: Option<Value> = sqlx::query_scalar(
        "SELECT input FROM pending_actions WHERE run_step_id = $1 AND tool = $2 \
         ORDER BY id LIMIT 1",
    )
    .bind(ctx.step_id())
    .bind(APPROVE_TOOL)
    .fetch_optional(&pool)
    .await
    .map_err(db("read the approval request"))?;
    let request = match asked {
        Some(asked) => asked,
        None => {
            type Draft = (
                PacketStatus,
                Option<i64>,
                sqlx::types::Json<Vec<PacketAnswer>>,
            );
            let draft: Option<Draft> = sqlx::query_as(
                "SELECT status, resume_version_id, answers FROM application_packets WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&pool)
            .await
            .map_err(db("read the packet"))?;
            match draft {
                Some((PacketStatus::Ready, resume, answers)) => draft_input(id, resume, &answers),
                _ => return Ok(json!({"packet_id": id, "approved": false})),
            }
        }
    };
    ctx.tool(APPROVE_TOOL, request).await?;
    Ok(json!({"packet_id": id, "approved": true}))
}

/// The approval workflow. Its input is `{"posting_id", "packet_id"}`.
pub struct ApprovePacket;

#[rocket::async_trait]
impl Workflow for ApprovePacket {
    fn name(&self) -> &'static str {
        APPROVE_WORKFLOW
    }

    fn steps(&self) -> &'static [&'static str] {
        &[APPROVE_STEP]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        match step {
            APPROVE_STEP => approve(ctx, input).await,
            other => Err(StepError::Fail(format!("no step is named {other:?}"))),
        }
    }
}

/// The [`ApproveApplicationPacket`] output.
#[derive(Debug, Serialize)]
pub struct PacketApproved {
    pub packet_id: i64,
}

/// What undoing an approval needs.
#[derive(Debug, Serialize, Deserialize)]
struct ApprovalUndo {
    packet_id: i64,
    approved_at: DateTime<Utc>,
}

/// `write`, gated ([`Tool::NEEDS_APPROVAL`]): approves a `ready` packet with the resume and
/// answers the user approved. The draft columns don't change. Undo returns the packet to
/// `ready` while it is still `approved` with the same `approved_at`.
pub struct ApproveApplicationPacket;

#[rocket::async_trait]
impl Tool for ApproveApplicationPacket {
    const NAME: &'static str = APPROVE_TOOL;
    const DESCRIPTION: &'static str =
        "Approves an application packet with the resume and answers the user approved";
    const PERMISSION: Permission = Permission::Write;
    const NEEDS_APPROVAL: bool = true;
    type Input = ApprovalInput;
    type Output = PacketApproved;

    fn input_schema() -> Value {
        let properties: serde_json::Map<String, Value> = QUESTIONS
            .iter()
            .map(|(key, _)| (key.as_str().to_owned(), json!({"type": ["string", "null"]})))
            .collect();
        let keys: Vec<&str> = QUESTIONS.iter().map(|(key, _)| key.as_str()).collect();
        json!({
            "type": "object",
            "properties": {
                "packet_id": {"type": "integer", "description": "The packet to approve"},
                "resume_version_id": {
                    "type": ["integer", "null"],
                    "description": "The resume to send"
                },
                "answers": {
                    "type": "object",
                    "properties": properties,
                    "required": keys,
                    "additionalProperties": false
                }
            },
            "required": ["packet_id", "resume_version_id", "answers"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"packet_id": {"type": "integer"}},
            "required": ["packet_id"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        ctx: &mut ToolContext<'_>,
        input: ApprovalInput,
    ) -> Result<PacketApproved, ToolError> {
        let id = input.packet_id;
        let what = "approve the packet";
        let mut tx = ctx.pool().begin().await.map_err(tool_db(what))?;
        type Row = (PacketStatus, String, sqlx::types::Json<Vec<PacketAnswer>>);
        let row: Option<Row> = sqlx::query_as(
            "SELECT p.status, j.company, p.answers FROM application_packets p \
             JOIN job_postings j ON j.id = p.posting_id WHERE p.id = $1 FOR UPDATE OF p",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(tool_db(what))?;
        let Some((status, company, draft)) = row else {
            return Err(ToolError::Failed(format!("packet {id} is gone")));
        };
        if status != PacketStatus::Ready {
            return Err(ToolError::Failed(format!(
                "packet {id} is {}, not ready",
                status.as_str()
            )));
        }
        let Some(resume) = input.resume_version_id else {
            return Err(ToolError::Failed("no resume was chosen".into()));
        };
        let name: Option<String> =
            sqlx::query_scalar("SELECT name FROM resume_versions WHERE id = $1 FOR SHARE")
                .bind(resume)
                .fetch_optional(&mut *tx)
                .await
                .map_err(tool_db(what))?;
        let Some(name) = name else {
            return Err(ToolError::Failed(format!("resume {resume} doesn't exist")));
        };
        if let Some(path) = too_long(&input.answers) {
            return Err(ToolError::Failed(format!(
                "{path}: at most {MAX_ANSWER} characters"
            )));
        }
        let answers = approved_answers(&draft, &input.answers, &company);
        let approved_at: DateTime<Utc> = sqlx::query_scalar(
            "UPDATE application_packets SET status = 'approved', \
             approved_resume_version_id = $2, approved_resume_name = $3, \
             approved_answers = $4, approved_at = now(), updated_at = now() \
             WHERE id = $1 RETURNING approved_at",
        )
        .bind(id)
        .bind(resume)
        .bind(&name)
        .bind(json!(answers))
        .fetch_one(&mut *tx)
        .await
        .map_err(tool_db(what))?;
        tx.commit().await.map_err(tool_db(what))?;
        ctx.set_undo(json!(ApprovalUndo {
            packet_id: id,
            approved_at,
        }));
        Ok(PacketApproved { packet_id: id })
    }

    async fn undo(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError> {
        let undo: ApprovalUndo = serde_json::from_value(data).map_err(|_| {
            UndoError::Conflict("the undo data doesn't fit approve_application_packet".into())
        })?;
        let id = undo.packet_id;
        let done = sqlx::query(
            "UPDATE application_packets SET status = 'ready', \
             approved_resume_version_id = NULL, approved_resume_name = NULL, \
             approved_answers = NULL, approved_at = NULL, updated_at = now() \
             WHERE id = $1 AND status = 'approved' AND approved_at = $2",
        )
        .bind(id)
        .bind(undo.approved_at)
        .execute(&mut *conn)
        .await?;
        if done.rows_affected() == 0 {
            return Err(UndoError::Conflict(format!(
                "packet {id} is gone, applied or approved again"
            )));
        }
        Ok(())
    }
}

/// The `apply` step: marks the packet applied through [`MarkPacketApplied`] and publishes
/// the application it created or updated. A rerun after the tool succeeded returns its
/// output.
async fn apply(ctx: &mut StepContext<'_>, input: Value) -> Result<Value, StepError> {
    let input = RunInput::deserialize(&input)
        .map_err(|err| StepError::Fail(format!("invalid input: {err}")))?;
    let id = input.packet_id;
    let pool = ctx.pool().clone();
    let done: Option<Option<Value>> = sqlx::query_scalar(
        "SELECT output FROM tool_calls WHERE run_step_id = $1 AND tool = $2 \
         AND status = 'succeeded' ORDER BY id DESC LIMIT 1",
    )
    .bind(ctx.step_id())
    .bind(APPLY_TOOL)
    .fetch_optional(&pool)
    .await
    .map_err(db("read the earlier call"))?;
    if let Some(Some(output)) = done {
        return Ok(output);
    }
    let status: Option<PacketStatus> =
        sqlx::query_scalar("SELECT status FROM application_packets WHERE id = $1")
            .bind(id)
            .fetch_optional(&pool)
            .await
            .map_err(db("read the packet"))?;
    let Some(status) = status else {
        return Err(StepError::Fail(format!("no packet {id}")));
    };
    if status != PacketStatus::Approved {
        return Err(StepError::Fail(format!(
            "packet {id} is {}, not approved",
            status.as_str()
        )));
    }
    let output = ctx
        .tool(APPLY_TOOL, json!({"run_id": ctx.run_id(), "packet_id": id}))
        .await?;
    let applied = PacketApplied::deserialize(&output)
        .map_err(|err| StepError::Fail(format!("unexpected tool output: {err}")))?;
    let application: Option<Application> = sqlx::query_as(&format!(
        "SELECT {} FROM applications WHERE id = $1",
        applications::COLUMNS
    ))
    .bind(applied.application_id)
    .fetch_optional(&pool)
    .await
    .map_err(db("read the application"))?;
    if let Some(application) = application {
        ctx.bus().publish(if applied.created {
            Event::ApplicationCreated { application }
        } else {
            Event::ApplicationUpdated { application }
        });
    }
    Ok(output)
}

/// The mark-applied workflow. Its input is `{"posting_id", "packet_id"}`.
pub struct ApplyPacket;

#[rocket::async_trait]
impl Workflow for ApplyPacket {
    fn name(&self) -> &'static str {
        APPLY_WORKFLOW
    }

    fn steps(&self) -> &'static [&'static str] {
        &[APPLY_STEP]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        match step {
            APPLY_STEP => apply(ctx, input).await,
            other => Err(StepError::Fail(format!("no step is named {other:?}"))),
        }
    }
}

/// The [`MarkPacketApplied`] input.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyInput {
    /// The running `apply_packet` run for the packet.
    pub run_id: i64,
    pub packet_id: i64,
}

/// The [`MarkPacketApplied`] output, and the `apply` step's.
#[derive(Debug, Serialize, Deserialize)]
pub struct PacketApplied {
    pub packet_id: i64,
    /// The application it created or updated.
    pub application_id: i64,
    /// Whether it created the application.
    pub created: bool,
}

/// An updated application's fields before mark applied changed them.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct Prior {
    status: ApplicationStatus,
    status_changed_at: DateTime<Utc>,
    resume_version_id: Option<i64>,
    updated_at: DateTime<Utc>,
}

/// What undoing mark applied needs.
#[derive(Debug, Serialize, Deserialize)]
struct AppliedUndo {
    packet_id: i64,
    application_id: i64,
    created: bool,
    applied_at: DateTime<Utc>,
    /// The application's `updated_at` after the change.
    updated_at: DateTime<Utc>,
    /// `null` for a created application.
    prior: Option<Prior>,
    /// The `status_changed` event it recorded, if any.
    event_id: Option<i64>,
}

/// The application's `source` for a posting from `board`.
fn board_name(source: &str) -> &'static str {
    match source {
        "greenhouse" => "Greenhouse",
        "lever" => "Lever",
        _ => "Ashby",
    }
}

/// `write`: marks an `approved` packet applied after the user submitted it themselves. It
/// creates the posting's application from the posting, with status `applied`, or updates
/// the one the posting's most recently applied packet links (its resume, and `sourced`
/// becomes `applied`). Only the running `apply_packet` run for the packet may call it. Undo
/// deletes a created application or restores an updated one, and returns the packet to
/// `approved`, while neither changed since.
pub struct MarkPacketApplied;

#[rocket::async_trait]
impl Tool for MarkPacketApplied {
    const NAME: &'static str = APPLY_TOOL;
    const DESCRIPTION: &'static str =
        "Records that the user applied with an approved application packet";
    const PERMISSION: Permission = Permission::Write;
    type Input = ApplyInput;
    type Output = PacketApplied;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {"type": "integer", "description": "The running apply_packet run"},
                "packet_id": {"type": "integer", "description": "The approved packet"}
            },
            "required": ["run_id", "packet_id"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "packet_id": {"type": "integer"},
                "application_id": {"type": "integer"},
                "created": {"type": "boolean"}
            },
            "required": ["packet_id", "application_id", "created"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        ctx: &mut ToolContext<'_>,
        input: ApplyInput,
    ) -> Result<PacketApplied, ToolError> {
        let id = input.packet_id;
        let what = "mark the packet applied";
        let mut tx = ctx.pool().begin().await.map_err(tool_db(what))?;
        let running: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM agent_runs WHERE id = $1 AND workflow = $2 \
             AND status = 'running' AND input -> 'packet_id' = to_jsonb($3::bigint))",
        )
        .bind(input.run_id)
        .bind(APPLY_WORKFLOW)
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(tool_db(what))?;
        if !running {
            return Err(ToolError::Failed(format!(
                "run {} is not a running apply_packet run for packet {id}",
                input.run_id
            )));
        }
        let packet: Option<(i64, PacketStatus, Option<i64>)> = sqlx::query_as(
            "SELECT posting_id, status, approved_resume_version_id FROM application_packets \
             WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(tool_db(what))?;
        let Some((posting_id, status, resume)) = packet else {
            return Err(ToolError::Failed(format!("packet {id} is gone")));
        };
        if status != PacketStatus::Approved {
            return Err(ToolError::Failed(format!(
                "packet {id} is {}, not approved",
                status.as_str()
            )));
        }
        let Some(resume) = resume else {
            return Err(ToolError::Failed(format!(
                "packet {id}'s approved resume was deleted"
            )));
        };
        // The posting's application: the one its most recently applied packet links.
        let linked: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT application_id FROM application_packets \
             WHERE posting_id = $1 AND status = 'applied' \
             ORDER BY applied_at DESC, id DESC LIMIT 1",
        )
        .bind(posting_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(tool_db(what))?;
        let prior: Option<(i64, Prior)> = match linked.flatten() {
            Some(application) => sqlx::query_as::<_, Prior>(
                "SELECT status, status_changed_at, resume_version_id, updated_at \
                 FROM applications WHERE id = $1 FOR UPDATE",
            )
            .bind(application)
            .fetch_optional(&mut *tx)
            .await
            .map_err(tool_db(what))?
            .map(|prior| (application, prior)),
            None => None,
        };

        let (application_id, created, event_id, updated_at) = match &prior {
            None => {
                let (company, title, url, location, source): (
                    String,
                    String,
                    String,
                    Option<String>,
                    String,
                ) = sqlx::query_as(
                    "SELECT company, title, url, location_raw, source FROM job_postings \
                     WHERE id = $1",
                )
                .bind(posting_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(tool_db(what))?;
                let url = (url.chars().count() <= MAX_URL && is_web_url(&url)).then_some(url);
                let (application, updated_at): (i64, DateTime<Utc>) = sqlx::query_as(
                    "INSERT INTO applications \
                     (company, title, url, location, source, status, resume_version_id) \
                     VALUES ($1, $2, $3, $4, $5, 'applied', $6) RETURNING id, updated_at",
                )
                .bind(head_chars(&company, MAX_SHORT))
                .bind(head_chars(&title, MAX_SHORT))
                .bind(url)
                .bind(location.map(|location| head_chars(&location, MAX_SHORT)))
                .bind(board_name(&source))
                .bind(resume)
                .fetch_one(&mut *tx)
                .await
                .map_err(tool_db(what))?;
                timeline::record(
                    &mut tx,
                    application,
                    EventKind::Created,
                    None,
                    Some(ApplicationStatus::Applied),
                    None,
                    None,
                )
                .await
                .map_err(tool_db(what))?;
                (application, true, None, updated_at)
            }
            Some((application, prior)) => {
                let sourced = prior.status == ApplicationStatus::Sourced;
                let updated_at: DateTime<Utc> = sqlx::query_scalar(
                    "UPDATE applications SET resume_version_id = $2, updated_at = now(), \
                     status = CASE WHEN $3 THEN 'applied' ELSE status END, \
                     status_changed_at = CASE WHEN $3 THEN now() ELSE status_changed_at END \
                     WHERE id = $1 RETURNING updated_at",
                )
                .bind(application)
                .bind(resume)
                .bind(sourced)
                .fetch_one(&mut *tx)
                .await
                .map_err(tool_db(what))?;
                let event = if sourced {
                    let event = timeline::record(
                        &mut tx,
                        *application,
                        EventKind::StatusChanged,
                        Some(ApplicationStatus::Sourced),
                        Some(ApplicationStatus::Applied),
                        None,
                        None,
                    )
                    .await
                    .map_err(tool_db(what))?;
                    Some(event.id)
                } else {
                    None
                };
                (*application, false, event, updated_at)
            }
        };
        let applied_at: DateTime<Utc> = sqlx::query_scalar(
            "UPDATE application_packets SET status = 'applied', application_id = $2, \
             applied_at = now(), updated_at = now() WHERE id = $1 RETURNING applied_at",
        )
        .bind(id)
        .bind(application_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(tool_db(what))?;
        tx.commit().await.map_err(tool_db(what))?;
        ctx.set_undo(json!(AppliedUndo {
            packet_id: id,
            application_id,
            created,
            applied_at,
            updated_at,
            prior: prior.map(|(_, prior)| prior),
            event_id,
        }));
        Ok(PacketApplied {
            packet_id: id,
            application_id,
            created,
        })
    }

    async fn undo(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError> {
        let undo: AppliedUndo = serde_json::from_value(data).map_err(|_| {
            UndoError::Conflict("the undo data doesn't fit mark_packet_applied".into())
        })?;
        let id = undo.packet_id;
        let application = undo.application_id;
        let packet: Option<(PacketStatus, Option<i64>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT status, application_id, applied_at FROM application_packets \
             WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let unchanged = packet.is_some_and(|(status, linked, applied_at)| {
            status == PacketStatus::Applied
                && linked == Some(application)
                && applied_at == Some(undo.applied_at)
        });
        if !unchanged {
            return Err(UndoError::Conflict(format!(
                "packet {id} is gone or changed since"
            )));
        }
        let updated_at: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT updated_at FROM applications WHERE id = $1 FOR UPDATE")
                .bind(application)
                .fetch_optional(&mut *conn)
                .await?;
        if updated_at != Some(undo.updated_at) {
            return Err(UndoError::Conflict(format!(
                "application {application} is gone or changed since"
            )));
        }
        sqlx::query(
            "UPDATE application_packets SET status = 'approved', application_id = NULL, \
             applied_at = NULL, updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
        if undo.created {
            sqlx::query("DELETE FROM applications WHERE id = $1")
                .bind(application)
                .execute(&mut *conn)
                .await?;
            return Ok(());
        }
        if let Some(prior) = undo.prior {
            sqlx::query(
                "UPDATE applications SET status = $2, status_changed_at = $3, \
                 resume_version_id = $4, updated_at = $5 WHERE id = $1",
            )
            .bind(application)
            .bind(prior.status.as_str())
            .bind(prior.status_changed_at)
            .bind(prior.resume_version_id)
            .bind(prior.updated_at)
            .execute(&mut *conn)
            .await?;
        }
        if let Some(event) = undo.event_id {
            sqlx::query("DELETE FROM application_events WHERE id = $1 AND application_id = $2")
                .bind(event)
                .bind(application)
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::score_fit::{CitationSource, FitCitation};

    fn cited(key: PacketQuestion, answer: Option<&str>) -> PacketAnswer {
        PacketAnswer {
            key,
            question: key.text("Old"),
            answer: answer.map(str::to_owned),
            citations: answer
                .map(|_| {
                    vec![FitCitation {
                        source: CitationSource::Memory,
                        memory_id: Some(1),
                        topic: None,
                        text: "memory".into(),
                    }]
                })
                .unwrap_or_default(),
        }
    }

    #[test]
    fn approved_answers_trim_blank_and_keep_unchanged_citations() {
        let draft = vec![
            cited(PacketQuestion::WorkAuthorization, Some("Yes")),
            cited(PacketQuestion::NoticePeriod, Some("Four weeks")),
            cited(PacketQuestion::SalaryExpectation, None),
            cited(PacketQuestion::WhyLeaving, Some("Scope")),
            cited(PacketQuestion::WhyCompany, Some("Rust")),
        ];
        let approved = ApprovalAnswers {
            work_authorization: Some("  Yes ".into()),
            notice_period: Some("Two weeks".into()),
            salary_expectation: Some("90k".into()),
            why_leaving: Some("   ".into()),
            why_company: None,
        };
        let answers = approved_answers(&draft, &approved, "Acme");
        let got: Vec<(PacketQuestion, Option<&str>, usize)> = answers
            .iter()
            .map(|answer| (answer.key, answer.answer.as_deref(), answer.citations.len()))
            .collect();
        assert_eq!(
            got,
            [
                (PacketQuestion::WorkAuthorization, Some("Yes"), 1),
                (PacketQuestion::NoticePeriod, Some("Two weeks"), 0),
                (PacketQuestion::SalaryExpectation, Some("90k"), 0),
                (PacketQuestion::WhyLeaving, None, 0),
                (PacketQuestion::WhyCompany, None, 0),
            ]
        );
        assert_eq!(answers[4].question, "Why do you want to work at Acme?");
    }

    #[test]
    fn draft_input_has_every_key_and_too_long_names_the_path() {
        let draft = vec![cited(PacketQuestion::NoticePeriod, Some("Soon"))];
        assert_eq!(
            draft_input(7, None, &draft),
            json!({"packet_id": 7, "resume_version_id": null, "answers": {
                "work_authorization": null, "notice_period": "Soon",
                "salary_expectation": null, "why_leaving": null, "why_company": null}})
        );
        let mut answers = ApprovalAnswers::of(&draft);
        assert_eq!(too_long(&answers), None);
        answers.why_leaving = Some(format!(" {} ", "a".repeat(MAX_ANSWER)));
        assert_eq!(too_long(&answers), None, "trimmed first");
        answers.why_leaving = Some("a".repeat(MAX_ANSWER + 1));
        assert_eq!(too_long(&answers).as_deref(), Some("$.answers.why_leaving"));
    }
}
