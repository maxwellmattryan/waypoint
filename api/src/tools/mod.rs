//! Typed agent tools and the registry that invokes them. A tool is a Rust type with a name,
//! a description, JSON schemas for its input and output, and a [`Permission`]. Every
//! invocation through [`Registry::invoke`] is recorded in `tool_calls` (see
//! docs/agent-runtime.md).
//!
//! Permission levels are stored on each call but not enforced yet; the approval gate for
//! `external` tools is a later job.

pub mod schema;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::Instant;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::claude::{
    ClaudeError, ClaudeRequest, ClaudeResponse, ClaudeRunner, MAX_ERROR_BYTES, head,
};

/// What a tool may do.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Permission {
    /// Reads data; no side effects.
    Read,
    /// Changes Waypoint's own data; reversible and logged.
    Write,
    /// Leaves Waypoint (sends, submits, calls out); always needs the user's approval.
    External,
}

impl Permission {
    /// The wire and database name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::External => "external",
        }
    }
}

/// Tokens and reported cost of successful model calls. `input_tokens` counts the call's
/// input, cache-creation and cache-read tokens; missing values count as 0.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

impl Spend {
    /// The spend of one successful call.
    pub fn of(response: &ClaudeResponse) -> Self {
        let usage = &response.usage;
        let count = |tokens: Option<i64>| tokens.unwrap_or(0).max(0);
        Self {
            input_tokens: count(usage.input_tokens)
                .saturating_add(count(usage.cache_creation_input_tokens))
                .saturating_add(count(usage.cache_read_input_tokens)),
            output_tokens: count(usage.output_tokens),
            cost_usd: response
                .cost_usd
                .filter(|cost| cost.is_finite())
                .unwrap_or(0.0)
                .max(0.0),
        }
    }

    pub fn add(&mut self, other: Spend) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cost_usd += other.cost_usd;
    }
}

/// Makes model calls and adds up what the successful ones spent. Failed calls return no
/// usage from the runner, so they aren't counted.
#[derive(Debug, Default)]
pub(crate) struct Meter {
    pub spend: Spend,
    /// The `claude_calls.id` of the most recent successful call.
    pub last_call: Option<i64>,
}

impl Meter {
    fn record(&mut self, response: &ClaudeResponse) {
        self.spend.add(Spend::of(response));
        if response.log_id.is_some() {
            self.last_call = response.log_id;
        }
    }

    pub async fn call(
        &mut self,
        pool: &PgPool,
        runner: &ClaudeRunner,
        req: &ClaudeRequest,
    ) -> Result<ClaudeResponse, ClaudeError> {
        let response = runner.call(pool, req).await?;
        self.record(&response);
        Ok(response)
    }

    pub async fn call_structured<T: DeserializeOwned>(
        &mut self,
        pool: &PgPool,
        runner: &ClaudeRunner,
        req: &ClaudeRequest,
        schema: &Value,
    ) -> Result<(T, ClaudeResponse), ClaudeError> {
        let (typed, response) = runner.call_structured(pool, req, schema).await?;
        self.record(&response);
        Ok((typed, response))
    }

    pub async fn call_with_mcp(
        &mut self,
        pool: &PgPool,
        runner: &ClaudeRunner,
        req: &ClaudeRequest,
        mcp_config: &Path,
    ) -> Result<ClaudeResponse, ClaudeError> {
        let response = runner.call_with_mcp(pool, req, mcp_config).await?;
        self.record(&response);
        Ok(response)
    }

    pub async fn call_structured_with_mcp<T: DeserializeOwned>(
        &mut self,
        pool: &PgPool,
        runner: &ClaudeRunner,
        req: &ClaudeRequest,
        schema: &Value,
        mcp_config: &Path,
    ) -> Result<(T, ClaudeResponse), ClaudeError> {
        let (typed, response) = runner
            .call_structured_with_mcp(pool, req, schema, mcp_config)
            .await?;
        self.record(&response);
        Ok((typed, response))
    }
}

/// What a tool's handler gets: the pool and metered model calls, whose spend is recorded
/// on the tool call (and added to the calling step).
pub struct ToolContext<'a> {
    pool: &'a PgPool,
    runner: &'a ClaudeRunner,
    meter: Meter,
}

impl ToolContext<'_> {
    pub fn pool(&self) -> &PgPool {
        self.pool
    }

    /// [`ClaudeRunner::call`], counted against this tool call.
    pub async fn call(&mut self, req: &ClaudeRequest) -> Result<ClaudeResponse, ClaudeError> {
        self.meter.call(self.pool, self.runner, req).await
    }

    /// [`ClaudeRunner::call_structured`], counted against this tool call.
    pub async fn call_structured<T: DeserializeOwned>(
        &mut self,
        req: &ClaudeRequest,
        schema: &Value,
    ) -> Result<(T, ClaudeResponse), ClaudeError> {
        self.meter
            .call_structured(self.pool, self.runner, req, schema)
            .await
    }
}

/// Why a tool's handler failed.
#[derive(Debug)]
pub enum ToolError {
    Failed(String),
    Claude(ClaudeError),
}

impl From<ClaudeError> for ToolError {
    fn from(err: ClaudeError) -> Self {
        Self::Claude(err)
    }
}

/// A typed agent tool. `Input` types should use `#[serde(deny_unknown_fields)]` so the
/// typed input refuses what the schema would let through. Handlers may run more than once
/// for the same step (steps are at-least-once), so they should be idempotent.
#[rocket::async_trait]
pub trait Tool: Send + Sync + 'static {
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    const PERMISSION: Permission;
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;

    /// The input's JSON schema, in the subset [`schema`] supports.
    fn input_schema() -> Value;
    /// The output's JSON schema, in the subset [`schema`] supports.
    fn output_schema() -> Value;

    async fn call(
        &self,
        ctx: &mut ToolContext<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolError>;
}

/// A registered tool, as the MCP server lists it (name, description and input schema).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub permission: Permission,
    pub input_schema: Value,
    pub output_schema: Value,
}

/// Why an invocation didn't produce a valid output.
#[derive(Debug)]
pub enum ToolCallError {
    /// No tool has the name.
    UnknownTool,
    /// The input broke the input schema or didn't fit the typed input.
    InvalidInput(String),
    /// The handler's output broke the output schema.
    InvalidOutput(String),
    /// The handler failed.
    Failed(String),
    /// A model call inside the handler failed.
    Claude(ClaudeError),
    /// The `tool_calls` row could not be written.
    Database(sqlx::Error),
}

impl fmt::Display for ToolCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool => write!(f, "no tool has this name"),
            Self::InvalidInput(message) => write!(f, "invalid tool input: {message}"),
            Self::InvalidOutput(message) => write!(f, "invalid tool output: {message}"),
            Self::Failed(message) => write!(f, "the tool failed: {message}"),
            Self::Claude(err) => write!(f, "the tool's model call failed: {err}"),
            Self::Database(err) => write!(f, "cannot record the tool call: {err}"),
        }
    }
}

impl std::error::Error for ToolCallError {}

/// The object-safe face of a [`Tool`]: JSON in, JSON out.
#[rocket::async_trait]
trait ErasedTool: Send + Sync {
    async fn call_json(
        &self,
        ctx: &mut ToolContext<'_>,
        input: Value,
    ) -> Result<Value, ToolCallError>;
}

#[rocket::async_trait]
impl<T: Tool> ErasedTool for T {
    async fn call_json(
        &self,
        ctx: &mut ToolContext<'_>,
        input: Value,
    ) -> Result<Value, ToolCallError> {
        // serde's message can quote the input, so only say that it didn't fit.
        let input = T::Input::deserialize(input).map_err(|_| {
            ToolCallError::InvalidInput("$: does not fit the tool's input type".into())
        })?;
        let output = self.call(ctx, input).await.map_err(|err| match err {
            ToolError::Failed(message) => ToolCallError::Failed(message),
            ToolError::Claude(err) => ToolCallError::Claude(err),
        })?;
        serde_json::to_value(output)
            .map_err(|err| ToolCallError::InvalidOutput(format!("$: cannot serialize: {err}")))
    }
}

struct Entry {
    info: ToolInfo,
    tool: Box<dyn ErasedTool>,
}

/// The tools agents may call, by name.
#[derive(Default)]
pub struct Registry {
    tools: BTreeMap<&'static str, Entry>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.tools.keys()).finish()
    }
}

/// How one invocation ended, before it is written to its `tool_calls` row.
struct Finished {
    status: &'static str,
    output: Option<Value>,
    result: Result<Value, ToolCallError>,
}

impl Registry {
    /// Adds `tool`. Refuses a name already registered and schemas outside the supported
    /// subset.
    pub fn register<T: Tool>(&mut self, tool: T) -> Result<(), String> {
        if self.tools.contains_key(T::NAME) {
            return Err(format!("a tool named {:?} is already registered", T::NAME));
        }
        let info = ToolInfo {
            name: T::NAME,
            description: T::DESCRIPTION,
            permission: T::PERMISSION,
            input_schema: T::input_schema(),
            output_schema: T::output_schema(),
        };
        schema::check_supported(&info.input_schema)
            .map_err(|err| format!("tool {:?} input schema: {err}", T::NAME))?;
        schema::check_supported(&info.output_schema)
            .map_err(|err| format!("tool {:?} output schema: {err}", T::NAME))?;
        self.tools.insert(
            T::NAME,
            Entry {
                info,
                tool: Box::new(tool),
            },
        );
        Ok(())
    }

    /// Every registered tool, by name.
    pub fn describe(&self) -> Vec<ToolInfo> {
        self.tools
            .values()
            .map(|entry| entry.info.clone())
            .collect()
    }

    /// The permission level of tool `name`, or `None` when no tool has the name.
    pub fn permission(&self, name: &str) -> Option<Permission> {
        self.tools.get(name).map(|entry| entry.info.permission)
    }

    /// Invokes tool `name` for run step `step_id` and records it in `tool_calls`.
    /// `claude_call_id` is the model call that asked for the tool, if any. Returns the
    /// output and what the handler's model calls spent.
    pub async fn invoke(
        &self,
        pool: &PgPool,
        runner: &ClaudeRunner,
        step_id: i64,
        claude_call_id: Option<i64>,
        name: &str,
        input: Value,
    ) -> Result<(Value, Spend), ToolCallError> {
        let (spend, _, result) = self
            .invoke_metered(pool, runner, step_id, claude_call_id, name, input)
            .await;
        result.map(|output| (output, spend))
    }

    /// [`Registry::invoke`], returning the spend on failure too, so a step counts a failed
    /// tool's model calls, and the `tool_calls.id` (`None` when the row couldn't be
    /// inserted).
    pub(crate) async fn invoke_metered(
        &self,
        pool: &PgPool,
        runner: &ClaudeRunner,
        step_id: i64,
        claude_call_id: Option<i64>,
        name: &str,
        input: Value,
    ) -> (Spend, Option<i64>, Result<Value, ToolCallError>) {
        let entry = self.tools.get(name);
        let inserted = sqlx::query_scalar(
            "INSERT INTO tool_calls (run_step_id, tool, permission, status, input, claude_call_id) \
             VALUES ($1, $2, $3, 'running', $4, $5) RETURNING id",
        )
        .bind(step_id)
        .bind(name)
        .bind(entry.map(|entry| entry.info.permission.as_str()))
        .bind(&input)
        .bind(claude_call_id)
        .fetch_one(pool)
        .await;
        let id: i64 = match inserted {
            Ok(id) => id,
            Err(err) => return (Spend::default(), None, Err(ToolCallError::Database(err))),
        };

        let started = Instant::now();
        let mut ctx = ToolContext {
            pool,
            runner,
            meter: Meter::default(),
        };
        let finished = match entry {
            None => Finished {
                status: "unknown_tool",
                output: None,
                result: Err(ToolCallError::UnknownTool),
            },
            Some(entry) => run(entry, &mut ctx, input).await,
        };
        let spend = ctx.meter.spend;
        let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let error = finished
            .result
            .as_ref()
            .err()
            .map(|err| head(&err.to_string(), MAX_ERROR_BYTES).to_owned());

        let written = sqlx::query(
            "UPDATE tool_calls SET status = $2, output = $3, error = $4, input_tokens = $5, \
             output_tokens = $6, cost_usd = $7, duration_ms = $8, finished_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(finished.status)
        .bind(&finished.output)
        .bind(error)
        .bind(spend.input_tokens)
        .bind(spend.output_tokens)
        .bind(spend.cost_usd)
        .bind(duration_ms)
        .execute(pool)
        .await;
        match written {
            Ok(_) => (spend, Some(id), finished.result),
            Err(err) => (spend, Some(id), Err(ToolCallError::Database(err))),
        }
    }
}

/// Checks the input, runs the handler and checks its output.
async fn run(entry: &Entry, ctx: &mut ToolContext<'_>, input: Value) -> Finished {
    if let Err(message) = schema::validate(&entry.info.input_schema, &input) {
        return Finished {
            status: "invalid_input",
            output: None,
            result: Err(ToolCallError::InvalidInput(message)),
        };
    }
    match entry.tool.call_json(ctx, input).await {
        Ok(output) => match schema::validate(&entry.info.output_schema, &output) {
            Ok(()) => Finished {
                status: "succeeded",
                output: Some(output.clone()),
                result: Ok(output),
            },
            Err(message) => Finished {
                status: "invalid_output",
                output: Some(output),
                result: Err(ToolCallError::InvalidOutput(message)),
            },
        },
        Err(err) => Finished {
            status: match err {
                ToolCallError::InvalidInput(_) => "invalid_input",
                ToolCallError::InvalidOutput(_) => "invalid_output",
                _ => "failed",
            },
            output: None,
            result: Err(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct Echo;

    #[rocket::async_trait]
    impl Tool for Echo {
        const NAME: &'static str = "echo";
        const DESCRIPTION: &'static str = "Returns its input";
        const PERMISSION: Permission = Permission::Read;
        type Input = Value;
        type Output = Value;

        fn input_schema() -> Value {
            json!({"type": "object"})
        }
        fn output_schema() -> Value {
            json!({"type": "object"})
        }
        async fn call(&self, _: &mut ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
            Ok(input)
        }
    }

    /// Its input schema uses an unsupported keyword.
    struct Bad(u8);

    #[rocket::async_trait]
    impl Tool for Bad {
        const NAME: &'static str = "bad";
        const DESCRIPTION: &'static str = "Uses unsupported keywords";
        const PERMISSION: Permission = Permission::External;
        type Input = Value;
        type Output = Value;

        fn input_schema() -> Value {
            json!({"type": "object", "properties": {"n": {"type": "integer", "minimum": 0}}})
        }
        fn output_schema() -> Value {
            json!({"anyOf": []})
        }
        async fn call(&self, _: &mut ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
            Ok(json!({"n": self.0, "input": input}))
        }
    }

    struct BadOutput;

    #[rocket::async_trait]
    impl Tool for BadOutput {
        const NAME: &'static str = "bad_output";
        const DESCRIPTION: &'static str = "Its output schema is unsupported";
        const PERMISSION: Permission = Permission::Write;
        type Input = Value;
        type Output = Value;

        fn input_schema() -> Value {
            json!({})
        }
        fn output_schema() -> Value {
            json!({"type": "object", "patternProperties": {}})
        }
        async fn call(&self, _: &mut ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
            Ok(input)
        }
    }

    #[test]
    fn register_rejects_bad_tools() {
        let mut registry = Registry::default();
        registry.register(Echo).expect("echo registers");

        let err = registry.register(Echo).unwrap_err();
        assert_eq!(err, "a tool named \"echo\" is already registered");

        let err = registry.register(Bad(1)).unwrap_err();
        assert_eq!(
            err,
            "tool \"bad\" input schema: $.properties.n: unsupported keyword \"minimum\""
        );

        let err = registry.register(BadOutput).unwrap_err();
        assert_eq!(
            err,
            "tool \"bad_output\" output schema: $: unsupported keyword \"patternProperties\""
        );

        let described = registry.describe();
        assert_eq!(described.len(), 1, "rejected tools are not registered");
        assert_eq!(
            described[0],
            ToolInfo {
                name: "echo",
                description: "Returns its input",
                permission: Permission::Read,
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
            }
        );
    }
}
