//! Retrieval: what a workflow step pulls from Waypoint's memories into its context (see
//! docs/agent-runtime.md). [`search`] finds confirmed memories with Postgres full-text
//! search over the `search` column the memories page uses (content, topic and tags,
//! English stemming), narrowed by tags and kinds. [`record`] stores what a step retrieved
//! in `run_memories`, with a snapshot of each memory, so the run inspector can show which
//! memories a run used. [`context`] renders retrieved memories as the block a
//! model-calling step puts in its prompt.
//!
//! Steps retrieve through [`crate::runs::StepContext::memories`], which searches and
//! records in one call. `tidy_application`'s `draft` step retrieves and records, then
//! appends the confirmed answer and preference memories' content to the message it
//! drafts, but makes no model call (the workflow stays model-free), so it never renders
//! [`context`]; the
//! first model-calling step that retrieves concatenates it into its `ClaudeRequest`
//! prompt.
//!
//! Matching is OR over the query's words, not `websearch_to_tsquery`'s default AND: a
//! step's query is a sentence, and requiring every word would almost never match.
//! Embeddings (pgvector) are deferred until the recall evals show full-text search
//! missing relevant memories.

use sqlx::PgExecutor;

use crate::memories::{COLUMNS, Memory, MemoryKind, normalise_tags};

/// Memories returned when a query gives no limit.
pub const DEFAULT_LIMIT: usize = 5;
/// Most memories one query returns.
pub const MAX_LIMIT: usize = 20;
/// Most query words searched; later words are ignored.
const MAX_WORDS: usize = 64;

/// What a step asks for.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Free text; a memory matches when it shares at least one stemmed, non-stop-word
    /// term with it.
    pub text: String,
    /// When not empty, a memory must carry at least one of these (normalised like stored
    /// tags).
    pub tags: Vec<String>,
    /// When not empty, a memory's kind must be one of these.
    pub kinds: Vec<MemoryKind>,
    /// [`DEFAULT_LIMIT`] when `None`; clamped to `1..=MAX_LIMIT`.
    pub limit: Option<usize>,
}

/// A memory a query returned, with its `ts_rank` score (0 for a tag-only query).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Retrieved {
    #[sqlx(flatten)]
    pub memory: Memory,
    pub score: f64,
}

/// The query's words: runs of alphanumeric characters, lowercased, without `or`,
/// deduplicated in order, at most [`MAX_WORDS`]. Only these reach
/// `websearch_to_tsquery`, so none of its operators (quotes, `-`) can.
fn words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.is_empty() || word == "or" || out.contains(&word) {
            continue;
        }
        out.push(word);
        if out.len() == MAX_WORDS {
            break;
        }
    }
    out
}

/// The limit a query gets: [`DEFAULT_LIMIT`] when absent, clamped to `1..=MAX_LIMIT`.
pub fn limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Confirmed memories matching `query`, best match first (`ts_rank`), then newest
/// `updated_at`, then `id` descending. Text with no usable terms (blank, punctuation or
/// stop words only) returns the memories carrying any of the query's tags with score 0,
/// or nothing when it has no tags. With neither words nor tags no query is run.
pub async fn search(executor: impl PgExecutor<'_>, query: &Query) -> sqlx::Result<Vec<Retrieved>> {
    let words = words(&query.text);
    let tags = normalise_tags(&query.tags);
    if words.is_empty() && tags.is_empty() {
        return Ok(Vec::new());
    }
    let kinds: Vec<&str> = query.kinds.iter().map(|kind| kind.as_str()).collect();
    let limit = i64::try_from(limit(query.limit)).unwrap_or(i64::MAX);
    // `websearch_to_tsquery` drops stop words, so the query can still be empty
    // (`numnode = 0`): then only the tags select.
    sqlx::query_as(&format!(
        "WITH q AS (SELECT websearch_to_tsquery('english', $1) AS q) \
         SELECT {COLUMNS}, \
         (CASE WHEN numnode(q.q) > 0 THEN ts_rank(search, q.q) ELSE 0 END)::float8 AS score \
         FROM memories, q \
         WHERE status = 'confirmed' \
         AND (CASE WHEN numnode(q.q) > 0 THEN search @@ q.q \
         ELSE cardinality($2::text[]) > 0 END) \
         AND (cardinality($2::text[]) = 0 OR tags && $2::text[]) \
         AND (cardinality($3::text[]) = 0 OR kind = ANY($3::text[])) \
         ORDER BY score DESC, updated_at DESC, id DESC \
         LIMIT $4"
    ))
    .bind(words.join(" or "))
    .bind(&tags)
    .bind(&kinds)
    .bind(limit)
    .fetch_all(executor)
    .await
}

/// Stores one `run_memories` row per memory for step `step_id`, in order, with a
/// snapshot of its kind, topic and content and its score. A memory the step already
/// recorded (an earlier attempt) is skipped. Returns how many rows were added.
pub async fn record(
    executor: impl PgExecutor<'_>,
    step_id: i64,
    memories: &[Retrieved],
) -> sqlx::Result<u64> {
    if memories.is_empty() {
        return Ok(0);
    }
    let ids: Vec<i64> = memories.iter().map(|m| m.memory.id).collect();
    let kinds: Vec<&str> = memories.iter().map(|m| m.memory.kind.as_str()).collect();
    let topics: Vec<&str> = memories.iter().map(|m| m.memory.topic.as_str()).collect();
    let contents: Vec<&str> = memories.iter().map(|m| m.memory.content.as_str()).collect();
    let scores: Vec<f64> = memories.iter().map(|m| m.score).collect();
    let done = sqlx::query(
        "INSERT INTO run_memories (run_id, run_step_id, memory_id, kind, topic, content, score) \
         SELECT s.run_id, s.id, m.memory_id, m.kind, m.topic, m.content, m.score \
         FROM run_steps s, \
         unnest($2::bigint[], $3::text[], $4::text[], $5::text[], $6::float8[]) \
         WITH ORDINALITY AS m (memory_id, kind, topic, content, score, ordinality) \
         WHERE s.id = $1 ORDER BY m.ordinality \
         ON CONFLICT DO NOTHING",
    )
    .bind(step_id)
    .bind(&ids)
    .bind(&kinds)
    .bind(&topics)
    .bind(&contents)
    .bind(&scores)
    .execute(executor)
    .await?;
    Ok(done.rows_affected())
}

/// The prompt block for retrieved memories: a heading line, then one line per memory in
/// order, `- [memory <id> · <kind> · <topic>] <content>`, with newlines in the content
/// replaced by spaces. Empty for no memories.
pub fn context(memories: &[Retrieved]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "What Waypoint knows about the user (confirmed memories, most relevant first):\n",
    );
    for Retrieved { memory, .. } in memories {
        let content = memory.content.replace(['\r', '\n'], " ");
        out.push_str(&format!(
            "- [memory {} · {} · {}] {content}\n",
            memory.id,
            memory.kind.as_str(),
            memory.topic.as_str()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::memories::{MemoryStatus, MemoryTopic, SourceKind};

    fn retrieved(id: i64, kind: MemoryKind, topic: MemoryTopic, content: &str) -> Retrieved {
        let now = Utc::now();
        Retrieved {
            memory: Memory {
                id,
                kind,
                topic,
                content: content.into(),
                tags: Vec::new(),
                status: MemoryStatus::Confirmed,
                confirmed_at: Some(now),
                source_kind: SourceKind::Manual,
                source_ref: None,
                source_label: None,
                created_at: now,
                updated_at: now,
            },
            score: 0.5,
        }
    }

    #[test]
    fn context_renders_one_line_per_memory() {
        assert_eq!(context(&[]), "");
        let memories = [
            retrieved(
                7,
                MemoryKind::Preference,
                MemoryTopic::Location,
                "Remote only",
            ),
            retrieved(
                3,
                MemoryKind::Answer,
                MemoryTopic::NoticePeriod,
                "Two weeks\nnegotiable",
            ),
        ];
        assert_eq!(
            context(&memories),
            "What Waypoint knows about the user (confirmed memories, most relevant first):\n\
             - [memory 7 · preference · location] Remote only\n\
             - [memory 3 · answer · notice_period] Two weeks negotiable\n"
        );
    }

    #[test]
    fn words_and_limit() {
        assert_eq!(
            words("Acme's \"Staff\" -Engineer, OR acme or Rust!"),
            ["acme", "s", "staff", "engineer", "rust"]
        );
        assert!(words("  -- !! ").is_empty());
        let many: String = (0..100).map(|n| format!("w{n} ")).collect();
        assert_eq!(words(&many).len(), MAX_WORDS);
        for (given, expected) in [(None, 5), (Some(0), 1), (Some(20), 20), (Some(99), 20)] {
            assert_eq!(limit(given), expected, "{given:?}");
        }
    }
}
