//! Goal and plan storage: reading a goal's detail, and [`write_version`], which writes a
//! new plan version with its five steps and applies the keep rule ([`kept_steps`]).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::types::Json;
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};

use super::{
    GOAL_COLUMNS, Goal, GoalDetail, GoalQuestion, GoalRun, MAX_GOALS, Plan, PlanAuthor,
    PlanCompany, PlanCompanyConfig, PlanConfig, PlanDropped, PlanStatus, PlanStep, PlanStepKind,
    PlanStepStatus, PlanTrigger, PlanVersion, ShortlistEntry,
};
use crate::sourcing::Source;
use crate::workflows::plan_goal;

/// The step kinds, in plan order.
pub const STEP_KINDS: [PlanStepKind; 5] = [
    PlanStepKind::TargetCompanies,
    PlanStepKind::Source,
    PlanStepKind::Score,
    PlanStepKind::Shortlist,
    PlanStepKind::Prep,
];

/// A plan version to write.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanDraft {
    pub author: PlanAuthor,
    pub trigger: PlanTrigger,
    pub status: PlanStatus,
    pub summary: String,
    pub confidence: Option<f64>,
    pub config: PlanConfig,
    pub dropped: PlanDropped,
    pub run_id: Option<i64>,
    pub review_item_id: Option<i64>,
    pub claude_call_id: Option<i64>,
    pub cost_usd: f64,
}

/// The plan [`write_version`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    pub id: i64,
    pub version: i32,
}

/// One `plans` row.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PlanRow {
    pub id: i64,
    pub goal_id: i64,
    pub version: i32,
    pub author: PlanAuthor,
    pub trigger: PlanTrigger,
    pub status: PlanStatus,
    pub summary: String,
    pub confidence: Option<f64>,
    pub config: Json<PlanConfig>,
    pub dropped: Json<PlanDropped>,
    pub run_id: Option<i64>,
    pub review_item_id: Option<i64>,
    pub claude_call_id: Option<i64>,
    pub cost_usd: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub(crate) const PLAN_COLUMNS: &str = "id, goal_id, version, author, trigger, status, summary, \
                                       confidence, config, dropped, run_id, review_item_id, \
                                       claude_call_id, cost_usd, created_at, updated_at";

/// One step of an earlier version, as the keep rule reads it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct StepRow {
    pub id: i64,
    pub kind: PlanStepKind,
    pub status: PlanStepStatus,
    pub config: Value,
    pub output: Option<Value>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Each step's config (see docs/goals.md, "Keep rule"). They are cumulative: each one
/// holds everything the steps before it ran on that changes what it does.
pub fn step_configs(config: &PlanConfig) -> [Value; 5] {
    let mut companies: Vec<&PlanCompanyConfig> = config.companies.iter().collect();
    companies.sort_by(|a, b| (a.source.as_str(), &a.board).cmp(&(b.source.as_str(), &b.board)));
    let companies: Vec<Value> = companies
        .iter()
        .map(|c| json!({"name": c.name, "source": c.source, "board": c.board}))
        .collect();
    let mut boards: Vec<String> = config
        .companies
        .iter()
        .map(|c| format!("{}/{}", c.source.as_str(), c.board))
        .collect();
    boards.sort();
    let lowered = |items: &[String]| {
        let mut items: Vec<String> = items.iter().map(|item| item.to_lowercase()).collect();
        items.sort();
        items
    };
    let target = json!({"companies": companies});
    let source = json!({"boards": boards});
    let mut score = source.clone();
    score["title_keywords"] = json!(lowered(&config.title_keywords));
    score["locations"] = json!(lowered(&config.locations));
    score["remote_only"] = json!(config.remote_only);
    score["max_to_score"] = json!(config.max_to_score);
    let mut shortlist = score.clone();
    shortlist["shortlist_size"] = json!(config.shortlist_size);
    shortlist["min_score"] = json!(config.min_score);
    let mut prep = shortlist.clone();
    prep["prep_notes"] = json!(config.prep_notes);
    [target, source, score, shortlist, prep]
}

/// The keep rule: for each new step, the index of the previous version's step it is
/// copied from, or `None` to start `pending`. Step `i` is kept when the previous step `i`
/// `succeeded` or was `skipped`, its config equals the new one, and every earlier step was
/// kept.
pub fn kept_steps(previous: &[StepRow], configs: &[Value; 5]) -> Vec<Option<usize>> {
    let mut keeping = true;
    configs
        .iter()
        .enumerate()
        .map(|(index, config)| {
            keeping = keeping
                && previous.get(index).is_some_and(|step| {
                    step.kind == STEP_KINDS[index]
                        && step.status.is_done()
                        && step.config == *config
                });
            keeping.then_some(index)
        })
        .collect()
}

/// Writes a new version of goal `goal_id`'s plan inside `tx` and makes it the goal's
/// current plan: numbers it, stores its five steps, copies the steps the keep rule keeps
/// (status, output, times and shortlist rows, with `kept_from` set) and starts the rest
/// `pending`. The caller holds the goal's row lock.
pub async fn write_version(
    tx: &mut Transaction<'_, Postgres>,
    goal_id: i64,
    draft: &PlanDraft,
) -> sqlx::Result<Written> {
    let previous: Option<i64> =
        sqlx::query_scalar("SELECT current_plan_id FROM goals WHERE id = $1")
            .bind(goal_id)
            .fetch_one(&mut **tx)
            .await?;
    let version: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) + 1 FROM plans WHERE goal_id = $1")
            .bind(goal_id)
            .fetch_one(&mut **tx)
            .await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO plans (goal_id, version, author, trigger, status, summary, confidence, \
         config, dropped, run_id, review_item_id, claude_call_id, cost_usd) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING id",
    )
    .bind(goal_id)
    .bind(version)
    .bind(draft.author.as_str())
    .bind(draft.trigger.as_str())
    .bind(draft.status.as_str())
    .bind(&draft.summary)
    .bind(draft.confidence)
    .bind(json!(draft.config))
    .bind(json!(draft.dropped))
    .bind(draft.run_id)
    .bind(draft.review_item_id)
    .bind(draft.claude_call_id)
    .bind(draft.cost_usd)
    .fetch_one(&mut **tx)
    .await?;

    let configs = step_configs(&draft.config);
    let previous_steps: Vec<StepRow> = match previous {
        Some(plan_id) => {
            sqlx::query_as(
                "SELECT id, kind, status, config, output, started_at, finished_at \
                 FROM plan_steps WHERE plan_id = $1 ORDER BY ordinal",
            )
            .bind(plan_id)
            .fetch_all(&mut **tx)
            .await?
        }
        None => Vec::new(),
    };
    let kept = kept_steps(&previous_steps, &configs);
    for (index, (kind, config)) in STEP_KINDS.iter().zip(configs).enumerate() {
        let ordinal = i32::try_from(index).unwrap_or_default();
        match kept[index].map(|at| &previous_steps[at]) {
            Some(old) => {
                let step: i64 = sqlx::query_scalar(
                    "INSERT INTO plan_steps (plan_id, ordinal, kind, status, config, output, \
                     kept_from, started_at, finished_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
                )
                .bind(id)
                .bind(ordinal)
                .bind(kind.as_str())
                .bind(old.status.as_str())
                .bind(&config)
                .bind(&old.output)
                .bind(old.id)
                .bind(old.started_at)
                .bind(old.finished_at)
                .fetch_one(&mut **tx)
                .await?;
                sqlx::query(
                    "INSERT INTO plan_shortlist (step_id, posting_id, rank, score, band) \
                     SELECT $1, posting_id, rank, score, band FROM plan_shortlist \
                     WHERE step_id = $2",
                )
                .bind(step)
                .bind(old.id)
                .execute(&mut **tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "INSERT INTO plan_steps (plan_id, ordinal, kind, status, config) \
                     VALUES ($1, $2, $3, 'pending', $4)",
                )
                .bind(id)
                .bind(ordinal)
                .bind(kind.as_str())
                .bind(&config)
                .execute(&mut **tx)
                .await?;
            }
        }
    }
    sqlx::query("UPDATE goals SET current_plan_id = $2, updated_at = now() WHERE id = $1")
        .bind(goal_id)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(Written { id, version })
}

/// One goal row.
pub async fn goal(db: impl PgExecutor<'_>, id: i64) -> sqlx::Result<Option<Goal>> {
    sqlx::query_as(&format!("SELECT {GOAL_COLUMNS} FROM goals WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

/// Up to [`MAX_GOALS`] goals, newest first.
pub async fn list(pool: &PgPool) -> sqlx::Result<Vec<Goal>> {
    sqlx::query_as(&format!(
        "SELECT {GOAL_COLUMNS} FROM goals ORDER BY id DESC LIMIT $1"
    ))
    .bind(MAX_GOALS)
    .fetch_all(pool)
    .await
}

/// One plan row.
pub async fn plan_row(db: impl PgExecutor<'_>, id: i64) -> sqlx::Result<Option<PlanRow>> {
    sqlx::query_as(&format!("SELECT {PLAN_COLUMNS} FROM plans WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

/// The `(source, board)` pairs among `companies` that have any `job_postings` row.
pub async fn known_boards(
    db: impl PgExecutor<'_>,
    companies: &[(Source, String)],
) -> sqlx::Result<HashSet<(Source, String)>> {
    if companies.is_empty() {
        return Ok(HashSet::new());
    }
    let sources: Vec<&str> = companies.iter().map(|(s, _)| s.as_str()).collect();
    let boards: Vec<&str> = companies.iter().map(|(_, b)| b.as_str()).collect();
    let found: Vec<(Source, String)> = sqlx::query_as(
        "SELECT DISTINCT p.source, p.board FROM job_postings p \
         JOIN unnest($1::text[], $2::text[]) AS c (source, board) \
         ON p.source = c.source AND p.board = c.board",
    )
    .bind(&sources)
    .bind(&boards)
    .fetch_all(db)
    .await?;
    Ok(found.into_iter().collect())
}

/// `companies` with `verified` set from `known`.
pub fn with_verified(
    companies: &[PlanCompanyConfig],
    known: &HashSet<(Source, String)>,
) -> Vec<PlanCompany> {
    companies
        .iter()
        .map(|c| PlanCompany {
            name: c.name.clone(),
            source: c.source,
            board: c.board.clone(),
            reason: c.reason.clone(),
            verified: known.contains(&(c.source, c.board.clone())),
        })
        .collect()
}

/// The board pairs of `companies`.
pub fn pairs(companies: &[PlanCompanyConfig]) -> Vec<(Source, String)> {
    companies
        .iter()
        .map(|c| (c.source, c.board.clone()))
        .collect()
}

/// Plan `id` with its steps, its shortlist and `verified` computed now.
pub async fn plan(pool: &PgPool, id: i64) -> sqlx::Result<Option<Plan>> {
    let Some(row) = plan_row(pool, id).await? else {
        return Ok(None);
    };
    let steps: Vec<PlanStep> = sqlx::query_as(
        "SELECT s.id, s.ordinal, s.kind, s.status, s.config, s.output, s.last_error, \
         s.kept_from, kp.version AS kept_from_version, s.started_at, s.finished_at \
         FROM plan_steps s LEFT JOIN plan_steps k ON k.id = s.kept_from \
         LEFT JOIN plans kp ON kp.id = k.plan_id WHERE s.plan_id = $1 ORDER BY s.ordinal",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let shortlist: Vec<ShortlistEntry> = sqlx::query_as(
        "SELECT l.rank, l.posting_id, p.company, p.title, l.score, l.band \
         FROM plan_shortlist l JOIN plan_steps s ON s.id = l.step_id \
         JOIN job_postings p ON p.id = l.posting_id \
         WHERE s.plan_id = $1 AND s.kind = 'shortlist' ORDER BY l.rank",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let config = row.config.0;
    let known = known_boards(pool, &pairs(&config.companies)).await?;
    Ok(Some(Plan {
        id: row.id,
        goal_id: row.goal_id,
        version: row.version,
        author: row.author,
        trigger: row.trigger,
        status: row.status,
        summary: row.summary,
        confidence: row.confidence,
        companies: with_verified(&config.companies, &known),
        title_keywords: config.title_keywords,
        locations: config.locations,
        remote_only: config.remote_only,
        max_to_score: config.max_to_score,
        shortlist_size: config.shortlist_size,
        min_score: config.min_score,
        prep_notes: config.prep_notes,
        dropped: row.dropped.0,
        run_id: row.run_id,
        review_item_id: row.review_item_id,
        claude_call_id: row.claude_call_id,
        cost_usd: row.cost_usd,
        created_at: row.created_at,
        updated_at: row.updated_at,
        steps,
        shortlist,
    }))
}

/// Goal `id` with its questions, current plan, versions and latest `plan_goal` run.
pub async fn detail(pool: &PgPool, id: i64) -> sqlx::Result<Option<GoalDetail>> {
    let Some(goal) = goal(pool, id).await? else {
        return Ok(None);
    };
    let questions: Vec<GoalQuestion> = sqlx::query_as(
        "SELECT id, ordinal, question, answer, answered_at FROM goal_questions \
         WHERE goal_id = $1 ORDER BY ordinal",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let plan = match goal.current_plan_id {
        Some(plan_id) => plan(pool, plan_id).await?,
        None => None,
    };
    let versions: Vec<PlanVersion> = sqlx::query_as(
        "SELECT id, version, author, trigger, status, summary, cost_usd, created_at \
         FROM plans WHERE goal_id = $1 ORDER BY version DESC",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let run: Option<GoalRun> = sqlx::query_as(
        "SELECT id, status, error, created_at FROM agent_runs WHERE workflow = $1 \
         AND input -> 'goal_id' = to_jsonb($2::bigint) ORDER BY id DESC LIMIT 1",
    )
    .bind(plan_goal::WORKFLOW)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(Some(GoalDetail {
        goal,
        questions,
        plan,
        versions,
        run,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PlanConfig {
        PlanConfig {
            companies: vec![
                PlanCompanyConfig {
                    name: "Beta".into(),
                    source: Source::Lever,
                    board: "beta".into(),
                    reason: Some("Climate".into()),
                },
                PlanCompanyConfig {
                    name: "Alpha".into(),
                    source: Source::Greenhouse,
                    board: "alpha".into(),
                    reason: None,
                },
            ],
            title_keywords: vec!["Engineer".into(), "Frontend".into()],
            locations: vec!["New York".into()],
            remote_only: false,
            max_to_score: 10,
            shortlist_size: 5,
            min_score: 60,
            prep_notes: "Lead with climate work".into(),
        }
    }

    fn previous(statuses: [PlanStepStatus; 5], configs: &[Value; 5]) -> Vec<StepRow> {
        statuses
            .iter()
            .zip(configs)
            .enumerate()
            .map(|(index, (status, config))| StepRow {
                id: 100 + i64::try_from(index).unwrap(),
                kind: STEP_KINDS[index],
                status: *status,
                config: config.clone(),
                output: Some(json!({"n": index})),
                started_at: None,
                finished_at: None,
            })
            .collect()
    }

    #[test]
    fn step_configs_are_cumulative_and_normalised() {
        let [target, source, score, shortlist, prep] = step_configs(&config());
        assert_eq!(
            target,
            json!({"companies": [
                {"name": "Alpha", "source": "greenhouse", "board": "alpha"},
                {"name": "Beta", "source": "lever", "board": "beta"},
            ]})
        );
        assert_eq!(
            source,
            json!({"boards": ["greenhouse/alpha", "lever/beta"]})
        );
        assert_eq!(score["title_keywords"], json!(["engineer", "frontend"]));
        assert_eq!(score["boards"], source["boards"]);
        assert_eq!(shortlist["max_to_score"], 10);
        assert_eq!(shortlist["min_score"], 60);
        assert_eq!(prep["min_score"], 60);
        assert_eq!(prep["prep_notes"], "Lead with climate work");
    }

    #[test]
    fn kept_steps_cases() {
        use PlanStepStatus::{Failed, Pending, Running, Skipped, Succeeded};
        let base = config();
        let base_configs = step_configs(&base);
        let changed = |patch: fn(&mut PlanConfig)| {
            let mut config = config();
            patch(&mut config);
            step_configs(&config)
        };
        let done = [Succeeded, Succeeded, Succeeded, Succeeded, Skipped];
        let all = vec![Some(0), Some(1), Some(2), Some(3), Some(4)];
        let none = vec![None; 5];

        // (name, previous statuses, new configs, expected)
        type Case = (
            &'static str,
            [PlanStepStatus; 5],
            [Value; 5],
            Vec<Option<usize>>,
        );
        let cases: Vec<Case> = vec![
            ("nothing changed", done, base_configs.clone(), all.clone()),
            (
                "only min_score changed keeps three steps",
                done,
                changed(|c| c.min_score = 40),
                vec![Some(0), Some(1), Some(2), None, None],
            ),
            (
                "only prep notes changed keeps four",
                done,
                changed(|c| c.prep_notes = "Other".into()),
                vec![Some(0), Some(1), Some(2), Some(3), None],
            ),
            (
                "the companies changed: nothing is kept",
                done,
                changed(|c| c.companies.truncate(1)),
                none.clone(),
            ),
            (
                "a company renamed: nothing is kept",
                done,
                changed(|c| c.companies[0].name = "Beta Inc".into()),
                none.clone(),
            ),
            (
                "a reason changed: everything is kept",
                done,
                changed(|c| c.companies[0].reason = None),
                all.clone(),
            ),
            (
                "keyword case and order don't matter",
                done,
                changed(|c| c.title_keywords = vec!["FRONTEND".into(), "engineer".into()]),
                all,
            ),
            (
                "a keyword changed resets score and after",
                done,
                changed(|c| c.title_keywords.push("Staff".into())),
                vec![Some(0), Some(1), None, None, None],
            ),
            (
                "a failed step and everything after it are never kept",
                [Succeeded, Succeeded, Failed, Pending, Pending],
                base_configs.clone(),
                vec![Some(0), Some(1), None, None, None],
            ),
            (
                "a running step isn't kept",
                [Succeeded, Running, Pending, Pending, Pending],
                base_configs.clone(),
                vec![Some(0), None, None, None, None],
            ),
            (
                "a failed first step keeps nothing",
                [Failed, Pending, Pending, Pending, Pending],
                base_configs.clone(),
                none,
            ),
        ];
        for (name, statuses, configs, expected) in cases {
            let previous = previous(statuses, &base_configs);
            assert_eq!(kept_steps(&previous, &configs), expected, "{name}");
        }
        assert_eq!(
            kept_steps(&[], &base_configs),
            vec![None; 5],
            "no previous version"
        );
    }
}
