//! Retrieval over memories: which confirmed memories a query returns and in what order,
//! the tag, kind and limit rules, and how a step records what it retrieved in
//! `run_memories`. No model is called.

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
use waypoint_api::events::{Event, EventBus};
use waypoint_api::jobs::{Wake, Worker};
use waypoint_api::memories::{
    self, Memory, MemoryInput, MemoryKind, MemorySource, MemoryStatus, MemoryTopic, NewMemory,
};
use waypoint_api::retrieval::{self, Query};
use waypoint_api::runs::{self, RunStatus, Runtime, StepContext, StepError, Workflow};
use waypoint_api::tools::Registry;

const TIMEOUT: Duration = Duration::from_secs(10);

/// Stores a memory with `status`, `updated_at` set `age` seconds before a fixed instant
/// so ties order predictably.
async fn add(
    pool: &PgPool,
    kind: MemoryKind,
    topic: MemoryTopic,
    content: &str,
    tags: &[&str],
    status: MemoryStatus,
    age: i64,
) -> Memory {
    let fields = memories::validate(&MemoryInput {
        kind,
        topic,
        content: content.into(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
    })
    .expect("a valid memory");
    let memory = memories::insert(
        pool,
        &NewMemory {
            fields,
            status,
            source: MemorySource::manual(),
        },
    )
    .await
    .expect("insert a memory");
    sqlx::query(
        "UPDATE memories SET updated_at = '2026-10-01T00:00:00Z'::timestamptz \
         - make_interval(secs => $2) WHERE id = $1",
    )
    .bind(memory.id)
    .bind(age as f64)
    .execute(pool)
    .await
    .expect("set updated_at");
    memory
}

async fn fact(pool: &PgPool, content: &str, tags: &[&str], age: i64) -> i64 {
    add(
        pool,
        MemoryKind::Fact,
        MemoryTopic::Skill,
        content,
        tags,
        MemoryStatus::Confirmed,
        age,
    )
    .await
    .id
}

async fn search(pool: &PgPool, query: Query) -> Vec<(i64, f64)> {
    retrieval::search(pool, &query)
        .await
        .expect("search")
        .into_iter()
        .map(|found| (found.memory.id, found.score))
        .collect()
}

async fn ids(pool: &PgPool, query: Query) -> Vec<i64> {
    search(pool, query)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

fn text(text: &str) -> Query {
    Query {
        text: text.into(),
        ..Default::default()
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

#[sqlx::test]
async fn search_matches_confirmed_memories_by_any_term(pool: PgPool) {
    // Two query terms in its content: the best match.
    let both = fact(&pool, "Senior Rust engineer", &[], 50).await;
    // One content term each; equal scores, so newest `updated_at`, then `id`, first.
    let older = fact(&pool, "Rust tooling", &[], 30).await;
    let newer = fact(&pool, "Rust tooling", &[], 10).await;
    let newer_twin = fact(&pool, "Rust tooling", &[], 10).await;
    // Topic only ("location" stems like "locations").
    let topic = add(
        &pool,
        MemoryKind::Preference,
        MemoryTopic::Location,
        "Berlin or Lisbon",
        &[],
        MemoryStatus::Confirmed,
        40,
    )
    .await
    .id;
    // Tag only.
    let tag = fact(&pool, "Prefers small teams", &["rust"], 20).await;
    // Matching words, but proposed; and no shared term.
    add(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Skill,
        "Rust engineer, proposed",
        &[],
        MemoryStatus::Proposed,
        0,
    )
    .await;
    fact(&pool, "Enjoys cooking", &[], 0).await;

    let found = search(
        &pool,
        Query {
            text: "Engineering in rust? Locations, or the \"best\" -ones".into(),
            limit: Some(20),
            ..Default::default()
        },
    )
    .await;
    let order: Vec<i64> = found.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        order,
        [both, newer_twin, newer, older, tag, topic],
        "{found:?}"
    );
    assert!(found.iter().all(|(_, score)| *score > 0.0), "{found:?}");
    assert!(found[0].1 > found[1].1, "two terms beat one: {found:?}");
    assert_eq!(found[1].1, found[3].1, "same content, same score");
    assert!(found[3].1 > found[5].1, "content outranks topic: {found:?}");

    // One shared word is enough; nothing shared finds nothing.
    assert_eq!(ids(&pool, text("cooking tooling")).await.len(), 4);
    assert!(ids(&pool, text("python haskell")).await.is_empty());
}

#[sqlx::test]
async fn search_filters_limits_and_empty_queries(pool: PgPool) {
    let remote = fact(&pool, "Rust services", &["Remote Work"], 10).await;
    let onsite = fact(&pool, "Rust services", &["onsite"], 20).await;
    let untagged = fact(&pool, "Rust services", &[], 30).await;
    let preference = add(
        &pool,
        MemoryKind::Preference,
        MemoryTopic::TargetRole,
        "Rust services",
        &["remote work"],
        MemoryStatus::Confirmed,
        40,
    )
    .await
    .id;
    // Tagged but no shared text; and tagged but proposed.
    let tagged_only = fact(&pool, "Cooking", &["remote work"], 5).await;
    add(
        &pool,
        MemoryKind::Fact,
        MemoryTopic::Skill,
        "Rust services",
        &["remote work"],
        MemoryStatus::Proposed,
        0,
    )
    .await;

    let with = |text: &str, tags: &[&str], kinds: &[MemoryKind]| Query {
        text: text.into(),
        tags: strings(tags),
        kinds: kinds.to_vec(),
        limit: Some(20),
    };
    // Any-of tags, normalised; filters only narrow what the text matched.
    assert_eq!(
        ids(&pool, with("rust", &["  REMOTE   work ", "onsite"], &[])).await,
        [remote, onsite, preference]
    );
    assert_eq!(
        ids(&pool, with("rust", &["remote work"], &[])).await,
        [remote, preference]
    );
    assert_eq!(
        ids(&pool, with("rust", &[], &[MemoryKind::Preference])).await,
        [preference]
    );
    assert_eq!(
        ids(
            &pool,
            with(
                "rust",
                &["remote work"],
                &[MemoryKind::Fact, MemoryKind::Answer]
            )
        )
        .await,
        [remote]
    );
    assert_eq!(
        ids(&pool, with("rust", &[], &[])).await,
        [remote, onsite, untagged, preference]
    );

    // No usable terms but tags: the tagged confirmed memories, newest first, score 0.
    for blank in ["", "  ?!  ", "the of and"] {
        assert_eq!(
            search(&pool, with(blank, &["Remote work"], &[])).await,
            [(tagged_only, 0.0), (remote, 0.0), (preference, 0.0)],
            "{blank:?}"
        );
    }
    // No usable terms and no tags: nothing.
    for blank in ["", "  ?!  ", "the of and", "or OR"] {
        assert!(
            search(&pool, with(blank, &[], &[])).await.is_empty(),
            "{blank:?}"
        );
    }

    // The limit defaults to 5 and clamps to 1..=20.
    for age in 0..25 {
        fact(&pool, "limit probe", &[], 100 + age).await;
    }
    let limited = |limit: Option<usize>| Query {
        text: "probe".into(),
        limit,
        ..Default::default()
    };
    for (limit, expected) in [(None, 5), (Some(0), 1), (Some(7), 7), (Some(100), 20)] {
        assert_eq!(
            ids(&pool, limited(limit)).await.len(),
            expected,
            "{limit:?}"
        );
    }
}

#[sqlx::test]
async fn by_kinds_returns_confirmed_memories_of_kinds(pool: PgPool) {
    let preference = |topic, content, status, age| {
        add(
            &pool,
            MemoryKind::Preference,
            topic,
            content,
            &[],
            status,
            age,
        )
    };
    let old = preference(
        MemoryTopic::DealBreaker,
        "No crypto",
        MemoryStatus::Confirmed,
        30,
    )
    .await
    .id;
    let new = preference(
        MemoryTopic::Location,
        "Remote only",
        MemoryStatus::Confirmed,
        10,
    )
    .await
    .id;
    let new_twin = preference(
        MemoryTopic::Seniority,
        "Staff or above",
        MemoryStatus::Confirmed,
        10,
    )
    .await
    .id;
    preference(
        MemoryTopic::Industry,
        "Proposed only",
        MemoryStatus::Proposed,
        0,
    )
    .await;
    let skill = fact(&pool, "Rust", &[], 20).await;
    let answer = add(
        &pool,
        MemoryKind::Answer,
        MemoryTopic::NoticePeriod,
        "Two weeks",
        &[],
        MemoryStatus::Confirmed,
        5,
    )
    .await
    .id;

    let found = |kinds: Vec<MemoryKind>, limit: usize| {
        let pool = pool.clone();
        async move {
            retrieval::by_kinds(&pool, &kinds, limit)
                .await
                .expect("by_kinds")
                .into_iter()
                .map(|found| (found.memory.id, found.score))
                .collect::<Vec<_>>()
        }
    };
    // Confirmed preferences only, newest `updated_at` then `id` first, score 0.
    assert_eq!(
        found(vec![MemoryKind::Preference], 30).await,
        [(new_twin, 0.0), (new, 0.0), (old, 0.0)]
    );
    // Several kinds; the limit applies, and isn't clamped to the search maximum.
    assert_eq!(
        found(vec![MemoryKind::Fact, MemoryKind::Answer], 30).await,
        [(answer, 0.0), (skill, 0.0)]
    );
    assert_eq!(found(vec![MemoryKind::Preference], 2).await.len(), 2);
    for age in 0..25 {
        fact(&pool, "many", &[], 100 + age).await;
    }
    assert_eq!(found(vec![MemoryKind::Fact], 30).await.len(), 26);
    // No kinds or a zero limit: nothing.
    assert!(found(vec![], 30).await.is_empty());
    assert!(found(vec![MemoryKind::Fact], 0).await.is_empty());
}

/// One step, `recall`, that retrieves memories for `input.text`. On its first attempt it
/// then adds a matching memory and asks for a retry, so the second attempt retrieves one
/// memory the first didn't.
struct Recall;

#[rocket::async_trait]
impl Workflow for Recall {
    fn name(&self) -> &'static str {
        "recall"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["recall"]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        _step: &str,
        input: Value,
    ) -> Result<Value, StepError> {
        let query = Query {
            text: input["text"].as_str().unwrap_or_default().into(),
            ..Default::default()
        };
        let found = ctx
            .memories(&query)
            .await
            .map_err(|err| StepError::Fail(err.to_string()))?;
        let late: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM memories WHERE content = 'Rust late')",
        )
        .fetch_one(ctx.pool())
        .await
        .map_err(|err| StepError::Fail(err.to_string()))?;
        if !late {
            fact(ctx.pool(), "Rust late", &[], 0).await;
            return Err(StepError::Retry("once more".into()));
        }
        Ok(json!({"found": found.len()}))
    }
}

/// `(memory_id, kind, topic, content)` of a run's memories, in API order.
async fn recorded(pool: &PgPool, run: i64) -> Vec<(Option<i64>, String, String, String)> {
    runs::get_detail(pool, run)
        .await
        .expect("read the run")
        .expect("the run exists")
        .memories
        .into_iter()
        .map(|m| {
            (
                m.memory_id,
                m.kind.as_str().to_owned(),
                m.topic.as_str().to_owned(),
                m.content,
            )
        })
        .collect()
}

#[sqlx::test]
async fn step_records_memories_once_and_keeps_snapshots(pool: PgPool) {
    let best = fact(&pool, "Rust and Go engineer", &[], 20).await;
    let other = fact(&pool, "Rust tooling", &[], 10).await;
    fact(&pool, "Unrelated cooking", &[], 0).await;

    let fake = common::fake("success", TIMEOUT);
    let bus = EventBus::new();
    let mut subscriber = bus.subscribe();
    let wake = Wake::default();
    let (_stop, stopped) = watch::channel(false);
    let runtime = Runtime::new(vec![Arc::new(Recall)], Registry::default()).expect("runtime");
    Worker {
        pool: pool.clone(),
        runner: fake.runner.clone(),
        bus: bus.clone(),
        config: common::fast_config(),
        wake: wake.0.clone(),
        runtime: runtime.clone(),
        google: waypoint_api::google::Google::new(Default::default()),
        capture: waypoint_api::capture::Capturer::new(Default::default()),
    }
    .spawn(stopped);
    let run = runs::start(
        &pool,
        &bus,
        &wake,
        &runtime,
        "recall",
        json!({"text": "rust go"}),
    )
    .await
    .expect("start the run");
    let events = common::events(&mut subscriber, TIMEOUT, |event| {
        matches!(event, Event::RunUpdated { run: r } if r.id == run.id && r.status == RunStatus::Succeeded)
    })
    .await;
    assert!(
        matches!(events.last(), Some(Event::RunUpdated { run: r }) if r.status == RunStatus::Succeeded),
        "{events:#?}"
    );

    let detail = runs::get_detail(&pool, run.id)
        .await
        .expect("read the run")
        .expect("the run exists");
    assert_eq!(detail.steps[0].step.attempts, 2);
    assert_eq!(detail.run.output, Some(json!({"found": 3})));
    let late: i64 = sqlx::query_scalar("SELECT id FROM memories WHERE content = 'Rust late'")
        .fetch_one(&pool)
        .await
        .expect("the late memory");
    // Both attempts retrieved `best` and `other`; they're recorded once, in rank order,
    // and the late memory is added by the second attempt.
    let rows: Vec<(Option<i64>, String, f64)> = detail
        .memories
        .iter()
        .map(|m| (m.memory_id, m.step.clone(), m.score))
        .collect();
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        [Some(best), Some(other), Some(late)],
        "{rows:?}"
    );
    assert!(
        rows.iter().all(|r| r.1 == "recall" && r.2 > 0.0),
        "{rows:?}"
    );
    assert!(
        detail
            .memories
            .iter()
            .all(|m| m.run_id == run.id && m.run_step_id == detail.steps[0].step.id)
    );

    // Editing or deleting a memory keeps the snapshot; deleting clears `memory_id`.
    sqlx::query("UPDATE memories SET content = 'Edited' WHERE id = $1")
        .bind(best)
        .execute(&pool)
        .await
        .expect("edit");
    sqlx::query("DELETE FROM memories WHERE id = $1")
        .bind(other)
        .execute(&pool)
        .await
        .expect("delete");
    assert_eq!(
        recorded(&pool, run.id).await,
        [
            (
                Some(best),
                "fact".into(),
                "skill".into(),
                "Rust and Go engineer".into()
            ),
            (None, "fact".into(), "skill".into(), "Rust tooling".into()),
            (
                Some(late),
                "fact".into(),
                "skill".into(),
                "Rust late".into()
            ),
        ]
    );

    // Deleting the run deletes its rows.
    sqlx::query("DELETE FROM agent_runs WHERE id = $1")
        .bind(run.id)
        .execute(&pool)
        .await
        .expect("delete the run");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM run_memories")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(left, 0);
}
