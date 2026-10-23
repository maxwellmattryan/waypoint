//! The workflows and tools registered in the running app (see docs/agent-runtime.md).

pub mod classify_email;
pub mod match_email;
pub mod memory_writer;
pub mod prep_packet;
pub mod propose_interview;
pub mod score_fit;
pub mod tidy;

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::PgPool;

use crate::evals::Suite;
use crate::google;
use crate::memories::{
    self, MemoryInput, MemoryKind, MemorySource, MemoryStatus, MemoryTopic, NewMemory,
};
use crate::runs::Runtime;
use crate::tools::Registry;

/// The production runtime with Google not configured, for the eval binary and tests (see
/// [`runtime_with`]).
pub fn runtime() -> Runtime {
    runtime_with(google::Google::new(google::Config::default()))
}

/// The production runtime: the `tidy_application` sample workflow and its tools, the
/// `memory_writer` and `classify_email` workflows (no tools), `match_email` with its two
/// `write` tools, `propose_interview` with its `external` tool, which writes to Google
/// Calendar through `google`, `score_fit` (no tools) and `prep_packet` with its one `write`
/// tool, `save_application_packet`.
pub fn runtime_with(google: google::Google) -> Runtime {
    let mut tools = Registry::default();
    tools
        .register(tidy::GetApplication)
        .expect("get_application registers");
    tools.register(tidy::AddNote).expect("add_note registers");
    tools
        .register(tidy::DraftFollowUp)
        .expect("draft_follow_up registers");
    tools
        .register(match_email::SetApplicationStatus)
        .expect("set_application_status registers");
    tools
        .register(match_email::AddApplicationStage)
        .expect("add_application_stage registers");
    tools
        .register(propose_interview::CreateCalendarEvent { google })
        .expect("create_calendar_event registers");
    tools
        .register(prep_packet::SaveApplicationPacket)
        .expect("save_application_packet registers");
    Runtime::new(
        vec![
            Arc::new(tidy::TidyApplication),
            Arc::new(memory_writer::MemoryWriter),
            Arc::new(classify_email::ClassifyEmail),
            Arc::new(match_email::MatchEmail),
            Arc::new(propose_interview::ProposeInterview),
            Arc::new(score_fit::ScoreFit),
            Arc::new(prep_packet::PrepPacket),
        ],
        tools,
    )
    .expect("a valid runtime")
}

/// The eval suites `cargo run --bin eval` can run, one per workflow (see docs/evals.md).
pub fn eval_suites() -> Vec<Arc<dyn Suite>> {
    vec![
        Arc::new(tidy::TidyEval),
        Arc::new(memory_writer::MemoryWriterEval),
        Arc::new(classify_email::ClassifyEmailEval),
        Arc::new(match_email::MatchEmailEval),
        Arc::new(propose_interview::ProposeInterviewEval),
        Arc::new(score_fit::ScoreFitEval),
        Arc::new(prep_packet::PrepPacketEval),
    ]
}

/// A memory an eval case seeds. Its id is the variable `memory_<name>`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvalMemory {
    name: String,
    pub(crate) kind: MemoryKind,
    topic: MemoryTopic,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    /// `confirmed` when absent.
    #[serde(default)]
    status: Option<MemoryStatus>,
}

/// Deletes every memory, then stores `memories` (source `manual`), adding `memory_<name>`
/// to `vars` for each, so a case starts with exactly its own memories.
pub(crate) async fn seed_memories(
    pool: &PgPool,
    memories: Vec<EvalMemory>,
    vars: &mut Map<String, Value>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM memories")
        .execute(pool)
        .await
        .map_err(|err| format!("cannot clear the memories: {err}"))?;
    for memory in memories {
        let name = memory.name.trim();
        if name.is_empty() {
            return Err("a memory has a blank name".into());
        }
        let var = format!("memory_{name}");
        if vars.contains_key(&var) {
            return Err(format!("two memories are named {name:?}"));
        }
        let fields = memories::validate(&MemoryInput {
            kind: memory.kind,
            topic: memory.topic,
            content: memory.content,
            tags: memory.tags,
        })
        .map_err(|err| {
            let fields: Vec<String> = err
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect();
            format!("memory {name:?} is invalid: {}", fields.join("; "))
        })?;
        let stored = memories::insert(
            pool,
            &NewMemory {
                fields,
                status: memory.status.unwrap_or(MemoryStatus::Confirmed),
                source: MemorySource::manual(),
            },
        )
        .await
        .map_err(|err| format!("cannot insert memory {name:?}: {err}"))?;
        vars.insert(var, json!(stored.id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Permission;

    #[test]
    fn runtime_registers_the_sample_workflow() {
        let runtime = runtime();
        let workflow = runtime
            .workflow("tidy_application")
            .expect("tidy_application is registered");
        assert_eq!(workflow.steps(), ["inspect", "note", "draft"]);
        let writer = runtime
            .workflow("memory_writer")
            .expect("memory_writer is registered");
        assert_eq!(writer.steps(), ["write"]);
        let classifier = runtime
            .workflow("classify_email")
            .expect("classify_email is registered");
        assert_eq!(classifier.steps(), ["classify"]);
        let matcher = runtime
            .workflow("match_email")
            .expect("match_email is registered");
        assert_eq!(matcher.steps(), ["match"]);
        let proposer = runtime
            .workflow("propose_interview")
            .expect("propose_interview is registered");
        assert_eq!(proposer.steps(), ["create"]);
        let scorer = runtime
            .workflow("score_fit")
            .expect("score_fit is registered");
        assert_eq!(scorer.steps(), ["score"]);
        let preparer = runtime
            .workflow("prep_packet")
            .expect("prep_packet is registered");
        assert_eq!(preparer.steps(), ["prepare"]);
        let suites: Vec<&str> = eval_suites().iter().map(|suite| suite.workflow()).collect();
        assert_eq!(
            suites,
            [
                "tidy_application",
                "memory_writer",
                "classify_email",
                "match_email",
                "propose_interview",
                "score_fit",
                "prep_packet"
            ]
        );

        let tools: Vec<(&str, Permission)> = runtime
            .tools()
            .describe()
            .into_iter()
            .map(|tool| (tool.name, tool.permission))
            .collect();
        assert_eq!(
            tools,
            [
                ("add_application_stage", Permission::Write),
                ("add_note", Permission::Write),
                ("create_calendar_event", Permission::External),
                ("draft_follow_up", Permission::External),
                ("get_application", Permission::Read),
                ("save_application_packet", Permission::Write),
                ("set_application_status", Permission::Write),
            ]
        );
    }
}
