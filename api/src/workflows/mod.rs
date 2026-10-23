//! The workflows and tools registered in the running app (see docs/agent-runtime.md).

pub mod classify_email;
pub mod match_email;
pub mod memory_writer;
pub mod tidy;

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::PgPool;

use crate::evals::Suite;
use crate::memories::{
    self, MemoryInput, MemoryKind, MemorySource, MemoryStatus, MemoryTopic, NewMemory,
};
use crate::runs::Runtime;
use crate::tools::Registry;

/// The production runtime: the `tidy_application` sample workflow and its tools, the
/// `memory_writer` and `classify_email` workflows (no tools), and `match_email` with its
/// two `write` tools.
pub fn runtime() -> Runtime {
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
    Runtime::new(
        vec![
            Arc::new(tidy::TidyApplication),
            Arc::new(memory_writer::MemoryWriter),
            Arc::new(classify_email::ClassifyEmail),
            Arc::new(match_email::MatchEmail),
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
    ]
}

/// A memory an eval case seeds. Its id is the variable `memory_<name>`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvalMemory {
    name: String,
    kind: MemoryKind,
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
        let suites: Vec<&str> = eval_suites().iter().map(|suite| suite.workflow()).collect();
        assert_eq!(
            suites,
            ["tidy_application", "memory_writer", "classify_email"]
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
                ("draft_follow_up", Permission::External),
                ("get_application", Permission::Read),
                ("set_application_status", Permission::Write),
            ]
        );
    }
}
