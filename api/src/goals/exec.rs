//! Plan execution: the `plan_step` job. Each job is one **tick** for one goal: it locks the
//! goal row for its whole run, walks the current plan from its first unfinished step, and
//! stops at a step that waits for `source_board` jobs or `score_fit` runs, at a failed step
//! (which replans, up to [`super::MAX_AUTO_REPLANS`] times) or at the end. The worker calls
//! [`job_settled`] when a `source_board` or `agent_run` job ends, which requests a tick for
//! every running goal waiting on it, and [`resume_running`] when it starts. See
//! docs/goals.md, "Execution".

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};

use super::store::{self, PlanRow};
use super::{
    GoalStatus, MAX_AUTO_REPLANS, PlanConfig, PlanStepKind, PlanStepStatus, lock_goal,
    queue_planner, set_status,
};
use crate::events::{Event, EventBus};
use crate::jobs::{self, AGENT_RUN, COLUMNS, Job, PLAN_STEP, SOURCE_BOARD, Wake};
use crate::runs::Run;
use crate::sourcing::{self, Source};
use crate::workflows::{plan_goal, score_fit};

/// The `prep` placeholder's output note.
pub const PREP_NOTE: &str = "Application prep isn't available yet";
/// The `target_companies` error for an empty list.
pub const NO_COMPANIES: &str = "No target companies";
/// The `score` error when no posting matches.
pub const NO_CANDIDATES: &str = "No live posting on the target boards matches the filters";

/// What one tick decided; `jobs::worker` maps it onto its outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done(Value),
    Retry(String),
    Fail(String),
}

/// Requests a tick for goal `goal_id` inside `tx`, whose caller holds the goal's row lock:
/// reuses a `queued` `plan_step` job for the goal (pulled forward to now) or enqueues one.
/// The caller commits, then wakes the worker.
pub async fn request_tick_tx(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    goal_id: i64,
) -> sqlx::Result<Job> {
    let queued: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE kind = $1 AND status = 'queued' \
         AND payload->>'goal_id' = $2::bigint::text ORDER BY id LIMIT 1",
    )
    .bind(PLAN_STEP)
    .bind(goal_id)
    .fetch_optional(&mut **tx)
    .await?;
    match queued {
        Some(id) => {
            sqlx::query_as(&format!(
                "UPDATE jobs SET run_at = LEAST(run_at, now()), updated_at = now() \
                 WHERE id = $1 RETURNING {COLUMNS}"
            ))
            .bind(id)
            .fetch_one(&mut **tx)
            .await
        }
        None => {
            jobs::enqueue_tx(
                tx,
                bus,
                PLAN_STEP,
                json!({"goal_id": goal_id}),
                jobs::Config::default().max_attempts,
            )
            .await
        }
    }
}

/// [`request_tick_tx`] in a transaction of its own that locks the goal first, then wakes
/// the worker.
pub async fn request_tick(
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    goal_id: i64,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    if lock_goal(&mut tx, goal_id).await?.is_none() {
        return Ok(());
    }
    request_tick_tx(&mut tx, bus, goal_id).await?;
    tx.commit().await?;
    wake.0.notify_one();
    Ok(())
}

/// The `plan_step` handler. Database errors retry; a tick for a goal that is gone or isn't
/// `running` succeeds without doing anything.
pub async fn run(pool: &PgPool, bus: &EventBus, wake: &Wake, job: &Job) -> Outcome {
    let Some(goal_id) = job.payload.get("goal_id").and_then(Value::as_i64) else {
        return Outcome::Fail(format!("invalid {PLAN_STEP} payload"));
    };
    match tick(pool, bus, wake, goal_id).await {
        Ok(result) => Outcome::Done(result),
        Err(err) => Outcome::Retry(format!("database error in goal {goal_id}: {err}")),
    }
}

/// What a tick changed, published after its commit.
#[derive(Default)]
struct Effects {
    changed: bool,
    wake: bool,
    runs: Vec<Run>,
}

/// What one step handler decided.
#[derive(Debug, Clone, PartialEq)]
enum StepResult {
    Succeeded(Value),
    Skipped(Value),
    Failed(String),
    /// Waiting for jobs or runs; a later tick goes on.
    Waiting,
}

#[derive(sqlx::FromRow)]
struct Step {
    id: i64,
    kind: PlanStepKind,
    status: PlanStepStatus,
    output: Option<Value>,
}

async fn tick(pool: &PgPool, bus: &EventBus, wake: &Wake, goal_id: i64) -> sqlx::Result<Value> {
    let mut tx = pool.begin().await?;
    let Some(goal) = lock_goal(&mut tx, goal_id).await? else {
        tx.commit().await?;
        return Ok(json!({"goal_id": goal_id, "result": "gone"}));
    };
    let plan = match goal.current_plan_id {
        Some(plan_id) if goal.status == GoalStatus::Running => {
            store::plan_row(&mut *tx, plan_id).await?
        }
        _ => None,
    };
    let Some(plan) = plan else {
        tx.commit().await?;
        return Ok(json!({"goal_id": goal_id, "result": "not running"}));
    };
    let steps: Vec<Step> = sqlx::query_as(
        "SELECT id, kind, status, output FROM plan_steps WHERE plan_id = $1 ORDER BY ordinal",
    )
    .bind(plan.id)
    .fetch_all(&mut *tx)
    .await?;

    let mut effects = Effects::default();
    let mut result = "completed";
    let mut score_output: Option<Value> = None;
    for step in steps {
        if step.status.is_done() {
            if step.kind == PlanStepKind::Score {
                score_output = step.output;
            }
            continue;
        }
        if step.status == PlanStepStatus::Pending {
            sqlx::query(
                "UPDATE plan_steps SET status = 'running', started_at = now(), \
                 updated_at = now() WHERE id = $1",
            )
            .bind(step.id)
            .execute(&mut *tx)
            .await?;
            effects.changed = true;
        }
        let config = &plan.config.0;
        let decided = match step.kind {
            PlanStepKind::TargetCompanies => target_companies(&mut tx, config).await?,
            PlanStepKind::Source => source(&mut tx, pool, bus, wake, step.id, config).await?,
            PlanStepKind::Score => {
                score(&mut tx, pool, bus, wake, step.id, step.output, config).await?
            }
            PlanStepKind::Shortlist => {
                shortlist(&mut tx, step.id, score_output.as_ref(), config).await?
            }
            PlanStepKind::Prep => StepResult::Skipped(json!({"note": PREP_NOTE})),
        };
        match decided {
            StepResult::Waiting => {
                result = "waiting";
                break;
            }
            StepResult::Succeeded(output) | StepResult::Skipped(output) => {
                let status = if matches!(step.kind, PlanStepKind::Prep) {
                    PlanStepStatus::Skipped
                } else {
                    PlanStepStatus::Succeeded
                };
                sqlx::query(
                    "UPDATE plan_steps SET status = $2, output = $3, last_error = NULL, \
                     finished_at = now(), updated_at = now() WHERE id = $1",
                )
                .bind(step.id)
                .bind(status.as_str())
                .bind(&output)
                .execute(&mut *tx)
                .await?;
                effects.changed = true;
                if step.kind == PlanStepKind::Score {
                    score_output = Some(output);
                }
            }
            StepResult::Failed(error) => {
                sqlx::query(
                    "UPDATE plan_steps SET status = 'failed', last_error = $2, \
                     finished_at = now(), updated_at = now() WHERE id = $1",
                )
                .bind(step.id)
                .bind(&error)
                .execute(&mut *tx)
                .await?;
                effects.changed = true;
                replan_or_fail(&mut tx, bus, &goal, &plan, step.kind, &error, &mut effects).await?;
                result = "failed";
                break;
            }
        }
    }
    if result == "completed" {
        set_status(&mut tx, goal_id, GoalStatus::Completed, None).await?;
        effects.changed = true;
    }
    tx.commit().await?;

    for run in effects.runs {
        bus.publish(Event::RunUpdated { run });
    }
    if effects.changed {
        super::publish(pool, bus, goal_id).await;
    }
    if effects.wake {
        wake.0.notify_one();
    }
    Ok(json!({"goal_id": goal_id, "plan_id": plan.id, "result": result}))
}

/// After step `kind` failed with `error`: below [`MAX_AUTO_REPLANS`] automatic replans,
/// counts one, sets the goal `planning` and queues a `step_failed` `plan_goal` run;
/// otherwise the goal fails with the step's error.
async fn replan_or_fail(
    tx: &mut Transaction<'_, Postgres>,
    bus: &EventBus,
    goal: &super::Goal,
    plan: &PlanRow,
    kind: PlanStepKind,
    error: &str,
    effects: &mut Effects,
) -> sqlx::Result<()> {
    if goal.auto_replans < MAX_AUTO_REPLANS {
        sqlx::query(
            "UPDATE goals SET auto_replans = auto_replans + 1, status = 'planning', \
             error = NULL, updated_at = now() WHERE id = $1",
        )
        .bind(goal.id)
        .execute(&mut **tx)
        .await?;
        let run = queue_planner(
            tx,
            bus,
            json!({
                "goal_id": goal.id, "trigger": "step_failed", "plan_id": plan.id,
                "step": kind.as_str(),
            }),
        )
        .await?;
        effects.runs.push(run);
        effects.wake = true;
    } else {
        let message = format!("{} failed: {error}", kind.as_str());
        set_status(tx, goal.id, GoalStatus::Failed, Some(&message)).await?;
    }
    Ok(())
}

/// `target_companies`: counts the companies and how many boards aren't verified yet.
async fn target_companies(
    tx: &mut Transaction<'_, Postgres>,
    config: &PlanConfig,
) -> sqlx::Result<StepResult> {
    if config.companies.is_empty() {
        return Ok(StepResult::Failed(NO_COMPANIES.into()));
    }
    let pairs = store::pairs(&config.companies);
    let known = store::known_boards(&mut **tx, &pairs).await?;
    let unverified = pairs.iter().filter(|pair| !known.contains(*pair)).count();
    Ok(StepResult::Succeeded(json!({
        "companies": config.companies.len(),
        "unverified": unverified,
    })))
}

#[derive(sqlx::FromRow)]
struct SourceTask {
    source: Option<String>,
    board: Option<String>,
    status: Option<String>,
    last_error: Option<String>,
    result: Option<Value>,
}

/// The `source` step's recorded fetches with their jobs' state.
async fn source_tasks(
    tx: &mut Transaction<'_, Postgres>,
    step_id: i64,
) -> sqlx::Result<Vec<SourceTask>> {
    sqlx::query_as(
        "SELECT t.source, t.board, j.status, j.last_error, j.result \
         FROM plan_step_tasks t LEFT JOIN jobs j ON j.id = t.job_id \
         WHERE t.step_id = $1 ORDER BY t.id",
    )
    .bind(step_id)
    .fetch_all(&mut **tx)
    .await
}

/// `source`: on its first tick, requests a fetch of every board (reusing one queued or
/// running) and records the jobs; then waits for them all to end.
async fn source(
    tx: &mut Transaction<'_, Postgres>,
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    step_id: i64,
    config: &PlanConfig,
) -> sqlx::Result<StepResult> {
    let mut tasks = source_tasks(tx, step_id).await?;
    if tasks.is_empty() {
        let mut seen = HashSet::new();
        for (source, board) in store::pairs(&config.companies) {
            if !seen.insert((source, board.clone())) {
                continue;
            }
            let requested = sourcing::request(pool, bus, wake, source, &board).await?;
            sqlx::query(
                "INSERT INTO plan_step_tasks (step_id, job_id, source, board) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(step_id)
            .bind(requested.job.id)
            .bind(source.as_str())
            .bind(&board)
            .execute(&mut **tx)
            .await?;
        }
        tasks = source_tasks(tx, step_id).await?;
    }
    if tasks.is_empty() {
        return Ok(StepResult::Failed(NO_COMPANIES.into()));
    }
    if tasks
        .iter()
        .any(|task| matches!(task.status.as_deref(), Some("queued" | "running")))
    {
        return Ok(StepResult::Waiting);
    }
    let failures: Vec<String> = tasks
        .iter()
        .filter(|task| task.status.as_deref() != Some("succeeded"))
        .map(|task| {
            format!(
                "Couldn't source {}/{}: {}",
                task.source.as_deref().unwrap_or_default(),
                task.board.as_deref().unwrap_or_default(),
                task.last_error.as_deref().unwrap_or("the fetch is gone"),
            )
        })
        .collect();
    if !failures.is_empty() {
        return Ok(StepResult::Failed(failures.join("; ")));
    }
    let sum = |key: &str| -> i64 {
        tasks
            .iter()
            .filter_map(|task| task.result.as_ref()?.get(key)?.as_i64())
            .sum()
    };
    Ok(StepResult::Succeeded(json!({
        "boards": tasks.len(),
        "inserted": sum("inserted"),
        "updated": sum("updated"),
    })))
}

/// What the `score` step stored on its first tick.
#[derive(Debug, Deserialize)]
struct ScoreProgress {
    posting_ids: Vec<i64>,
    reused: i64,
    in_review: i64,
}

#[derive(sqlx::FromRow)]
struct Existing {
    id: i64,
    applied: bool,
    in_review: bool,
}

/// `score`: on its first tick, picks the candidates, queues a `score_fit` run for each one
/// that has neither an applied score nor one waiting in review, and stores its progress;
/// then waits for the queued runs to end.
#[allow(clippy::too_many_arguments)]
async fn score(
    tx: &mut Transaction<'_, Postgres>,
    pool: &PgPool,
    bus: &EventBus,
    wake: &Wake,
    step_id: i64,
    progress: Option<Value>,
    config: &PlanConfig,
) -> sqlx::Result<StepResult> {
    let progress = match progress.and_then(|value| ScoreProgress::deserialize(value).ok()) {
        Some(progress) => progress,
        None => {
            let candidates = candidates(tx, config).await?;
            if candidates.is_empty() {
                return Ok(StepResult::Failed(NO_CANDIDATES.into()));
            }
            let existing: Vec<Existing> = sqlx::query_as(
                "SELECT c.id, \
                 EXISTS (SELECT 1 FROM posting_fit_scores WHERE posting_id = c.id \
                   AND status = 'applied') AS applied, \
                 COALESCE((SELECT s.status = 'in_review' AND r.status = 'pending' \
                   FROM posting_fit_scores s LEFT JOIN review_items r ON r.id = s.review_item_id \
                   WHERE s.posting_id = c.id ORDER BY s.id DESC LIMIT 1), false) AS in_review \
                 FROM unnest($1::bigint[]) WITH ORDINALITY AS c (id, n) ORDER BY c.n",
            )
            .bind(&candidates)
            .fetch_all(&mut **tx)
            .await?;
            let (mut reused, mut in_review) = (0, 0);
            for posting in &existing {
                if posting.applied {
                    reused += 1;
                } else if posting.in_review {
                    in_review += 1;
                } else if let Some((run, _)) =
                    score_fit::request(pool, bus, wake, posting.id).await?
                {
                    sqlx::query(
                        "INSERT INTO plan_step_tasks (step_id, run_id, posting_id) \
                         VALUES ($1, $2, $3)",
                    )
                    .bind(step_id)
                    .bind(run.id)
                    .bind(posting.id)
                    .execute(&mut **tx)
                    .await?;
                }
            }
            let progress = json!({
                "posting_ids": candidates, "reused": reused, "in_review": in_review,
            });
            sqlx::query("UPDATE plan_steps SET output = $2, updated_at = now() WHERE id = $1")
                .bind(step_id)
                .bind(&progress)
                .execute(&mut **tx)
                .await?;
            ScoreProgress {
                posting_ids: candidates,
                reused,
                in_review,
            }
        }
    };
    let runs: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT r.status, r.error FROM plan_step_tasks t \
         LEFT JOIN agent_runs r ON r.id = t.run_id WHERE t.step_id = $1 ORDER BY t.id",
    )
    .bind(step_id)
    .fetch_all(&mut **tx)
    .await?;
    let settled =
        |status: Option<&str>| matches!(status, None | Some("succeeded" | "failed" | "rejected"));
    if runs.iter().any(|(status, _)| !settled(status.as_deref())) {
        return Ok(StepResult::Waiting);
    }
    let failed: Vec<&(Option<String>, Option<String>)> = runs
        .iter()
        .filter(|(status, _)| status.as_deref() != Some("succeeded"))
        .collect();
    let queued = runs.len();
    if queued > 0 && failed.len() == queued && progress.reused == 0 && progress.in_review == 0 {
        let first = failed
            .first()
            .and_then(|(_, error)| error.clone())
            .unwrap_or_else(|| "the run is gone".into());
        return Ok(StepResult::Failed(format!(
            "Every fit score failed: {first}"
        )));
    }
    Ok(StepResult::Succeeded(json!({
        "posting_ids": progress.posting_ids,
        "queued": queued,
        "reused": progress.reused,
        "in_review": progress.in_review,
        "failed": failed.len(),
    })))
}

/// The live postings on the plan's boards that pass its filters, newest first, at most
/// `max_to_score`.
async fn candidates(
    tx: &mut Transaction<'_, Postgres>,
    config: &PlanConfig,
) -> sqlx::Result<Vec<i64>> {
    let pairs = store::pairs(&config.companies);
    let sources: Vec<&str> = pairs.iter().map(|(s, _)| Source::as_str(*s)).collect();
    let boards: Vec<&str> = pairs.iter().map(|(_, b)| b.as_str()).collect();
    sqlx::query_scalar(
        "SELECT p.id FROM job_postings p \
         WHERE p.removed_at IS NULL \
         AND EXISTS (SELECT 1 FROM unnest($1::text[], $2::text[]) AS b (source, board) \
           WHERE b.source = p.source AND b.board = p.board) \
         AND EXISTS (SELECT 1 FROM unnest($3::text[]) AS k (word) \
           WHERE strpos(lower(p.title), lower(k.word)) > 0) \
         AND (cardinality($4::text[]) = 0 OR EXISTS (SELECT 1 FROM unnest($4::text[]) AS l (place) \
           WHERE strpos(lower(coalesce(p.location_raw, '')), lower(l.place)) > 0 \
           OR strpos(lower(coalesce(p.city, '')), lower(l.place)) > 0 \
           OR strpos(lower(coalesce(p.region, '')), lower(l.place)) > 0 \
           OR strpos(lower(coalesce(p.country, '')), lower(l.place)) > 0)) \
         AND (NOT $5 OR p.remote) \
         ORDER BY p.posted_at DESC NULLS LAST, p.id DESC LIMIT $6",
    )
    .bind(&sources)
    .bind(&boards)
    .bind(&config.title_keywords)
    .bind(&config.locations)
    .bind(config.remote_only)
    .bind(i64::from(config.max_to_score))
    .fetch_all(&mut **tx)
    .await
}

/// `shortlist`: the score step's postings whose current applied score is at least
/// `min_score`, best first, at most `shortlist_size`, stored with their rank.
async fn shortlist(
    tx: &mut Transaction<'_, Postgres>,
    step_id: i64,
    score_output: Option<&Value>,
    config: &PlanConfig,
) -> sqlx::Result<StepResult> {
    let posting_ids: Vec<i64> = score_output
        .and_then(|output| output.get("posting_ids"))
        .and_then(|ids| Vec::<i64>::deserialize(ids).ok())
        .unwrap_or_default();
    let picked: Vec<(i64, i32)> = sqlx::query_as(
        "SELECT p.id, fit.score FROM job_postings p \
         JOIN LATERAL (SELECT score FROM posting_fit_scores WHERE posting_id = p.id \
           AND status = 'applied' ORDER BY updated_at DESC, id DESC LIMIT 1) fit ON true \
         WHERE p.id = ANY($1) AND fit.score >= $2 \
         ORDER BY fit.score DESC, p.id DESC LIMIT $3",
    )
    .bind(&posting_ids)
    .bind(config.min_score)
    .bind(i64::from(config.shortlist_size))
    .fetch_all(&mut **tx)
    .await?;
    let in_review: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_postings p WHERE p.id = ANY($1) AND EXISTS ( \
           SELECT 1 FROM posting_fit_scores s JOIN review_items r ON r.id = s.review_item_id \
           AND r.status = 'pending' WHERE s.posting_id = p.id AND s.status = 'in_review' \
           AND s.id > COALESCE((SELECT max(id) FROM posting_fit_scores \
             WHERE posting_id = p.id AND status = 'applied'), 0))",
    )
    .bind(&posting_ids)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM plan_shortlist WHERE step_id = $1")
        .bind(step_id)
        .execute(&mut **tx)
        .await?;
    if picked.is_empty() {
        return Ok(StepResult::Failed(format!(
            "No posting scored {} or more ({in_review} waiting in review)",
            config.min_score
        )));
    }
    for (rank, (posting_id, score)) in picked.iter().enumerate() {
        sqlx::query(
            "INSERT INTO plan_shortlist (step_id, posting_id, rank, score, band) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(step_id)
        .bind(posting_id)
        .bind(i32::try_from(rank + 1).unwrap_or(i32::MAX))
        .bind(score)
        .bind(score_fit::FitBand::of(*score).as_str())
        .execute(&mut **tx)
        .await?;
    }
    Ok(StepResult::Succeeded(json!({
        "shortlisted": picked.len(),
        "in_review": in_review,
    })))
}

/// The worker's hook after it recorded a terminal outcome of `job`. For a `source_board`
/// or `agent_run` job it requests a tick for every `running` goal whose current plan waits
/// on it, and fails a `planning` goal whose latest `plan_goal` run just failed. Best
/// effort: errors are logged.
pub async fn job_settled(pool: &PgPool, bus: &EventBus, wake: &Wake, job: &Job) {
    if job.kind != SOURCE_BOARD && job.kind != AGENT_RUN {
        return;
    }
    if let Err(err) = settled(pool, bus, wake, job).await {
        rocket::warn!("goals: cannot react to job {}: {err}", job.id);
    }
}

#[derive(sqlx::FromRow)]
struct FailedPlanner {
    id: i64,
    input: Value,
    error: Option<String>,
}

async fn settled(pool: &PgPool, bus: &EventBus, wake: &Wake, job: &Job) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    // Waits for any tick holding a goal, so the task rows it wrote are visible below.
    sqlx::query("SELECT id FROM goals WHERE status = 'running' ORDER BY id FOR UPDATE")
        .fetch_all(&mut *tx)
        .await?;
    let waiting: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT g.id FROM goals g \
         JOIN plan_steps s ON s.plan_id = g.current_plan_id \
         JOIN plan_step_tasks t ON t.step_id = s.id \
         LEFT JOIN agent_runs r ON r.id = t.run_id \
         WHERE g.status = 'running' AND (t.job_id = $1 OR r.job_id = $1) ORDER BY g.id",
    )
    .bind(job.id)
    .fetch_all(&mut *tx)
    .await?;
    for goal_id in &waiting {
        request_tick_tx(&mut tx, bus, *goal_id).await?;
    }

    let mut failed_goal = None;
    if job.kind == AGENT_RUN {
        let planner: Option<FailedPlanner> = sqlx::query_as(
            "SELECT id, input, error FROM agent_runs \
             WHERE job_id = $1 AND workflow = $2 AND status = 'failed'",
        )
        .bind(job.id)
        .bind(plan_goal::WORKFLOW)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(run) = planner
            && let Some(goal_id) = run.input.get("goal_id").and_then(Value::as_i64)
            && let Some(goal) = lock_goal(&mut tx, goal_id).await?
            && goal.status == GoalStatus::Planning
        {
            let latest: Option<i64> = sqlx::query_scalar(
                "SELECT max(id) FROM agent_runs WHERE workflow = $1 \
                 AND input -> 'goal_id' = to_jsonb($2::bigint)",
            )
            .bind(plan_goal::WORKFLOW)
            .bind(goal_id)
            .fetch_one(&mut *tx)
            .await?;
            if latest == Some(run.id) {
                let message = format!(
                    "Planning failed: {}",
                    run.error.as_deref().unwrap_or("the planner run failed")
                );
                set_status(&mut tx, goal_id, GoalStatus::Failed, Some(&message)).await?;
                failed_goal = Some(goal_id);
            }
        }
    }
    tx.commit().await?;
    // Also starts a tick a finished `plan_goal` run requested.
    wake.0.notify_one();
    if let Some(goal_id) = failed_goal {
        super::publish(pool, bus, goal_id).await;
    }
    Ok(())
}

/// Requests a tick for every `running` goal, so a crash never strands a plan. Called when
/// the worker starts. Best effort: errors are logged.
pub async fn resume_running(pool: &PgPool, bus: &EventBus, wake: &Wake) {
    let running: Vec<i64> =
        match sqlx::query_scalar("SELECT id FROM goals WHERE status = 'running' ORDER BY id")
            .fetch_all(pool)
            .await
        {
            Ok(running) => running,
            Err(err) => {
                rocket::warn!("goals: cannot list running goals: {err}");
                return;
            }
        };
    for goal_id in running {
        if let Err(err) = request_tick(pool, bus, wake, goal_id).await {
            rocket::warn!("goals: cannot resume goal {goal_id}: {err}");
        }
    }
}
