//! The workflows and tools registered in the running app (see docs/agent-runtime.md).

pub mod tidy;

use std::sync::Arc;

use crate::evals::Suite;
use crate::runs::Runtime;
use crate::tools::Registry;

/// The production runtime: the `tidy_application` sample workflow and its tools.
pub fn runtime() -> Runtime {
    let mut tools = Registry::default();
    tools
        .register(tidy::GetApplication)
        .expect("get_application registers");
    tools.register(tidy::AddNote).expect("add_note registers");
    tools
        .register(tidy::DraftFollowUp)
        .expect("draft_follow_up registers");
    Runtime::new(vec![Arc::new(tidy::TidyApplication)], tools).expect("a valid runtime")
}

/// The eval suites `cargo run --bin eval` can run, one per workflow (see docs/evals.md).
pub fn eval_suites() -> Vec<Arc<dyn Suite>> {
    vec![Arc::new(tidy::TidyEval)]
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

        let tools: Vec<(&str, Permission)> = runtime
            .tools()
            .describe()
            .into_iter()
            .map(|tool| (tool.name, tool.permission))
            .collect();
        assert_eq!(
            tools,
            [
                ("add_note", Permission::Write),
                ("draft_follow_up", Permission::External),
                ("get_application", Permission::Read),
            ]
        );
    }
}
