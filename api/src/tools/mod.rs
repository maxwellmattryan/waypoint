//! Typed agent tools and the registry that invokes them. A tool is a Rust type with a name,
//! a description, JSON schemas for its input and output, and a [`Permission`]. Every
//! invocation through [`Registry::invoke`] is recorded in `tool_calls` (see
//! docs/agent-runtime.md).
//!
//! Permission levels are enforced here, for step code and the MCP server alike:
//!
//! - an `external` call never runs its handler on first request. It waits in
//!   `pending_actions` for the user's decision ([`ToolCallError::AwaitingApproval`]), and
//!   the step repeating the same request later runs the approved input once,
//! - a succeeded `write` call is logged in `agent_actions` with the data its
//!   [`Tool::undo`] needs, set by the handler through [`ToolContext::set_undo`].

pub mod schema;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::Instant;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};

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
    undo: Option<Value>,
}

impl ToolContext<'_> {
    pub fn pool(&self) -> &PgPool {
        self.pool
    }

    /// Records what [`Tool::undo`] needs to revert this call. A `write` tool's handler
    /// calls it once its change is made; the data is stored on the call's `agent_actions`
    /// row. Without it the action is logged but can't be undone.
    pub fn set_undo(&mut self, data: Value) {
        self.undo = Some(data);
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

/// Why a tool's undo didn't revert an action.
#[derive(Debug)]
pub enum UndoError {
    /// The tool has no undo (the default for every tool).
    Unsupported,
    /// The record changed or is gone, so reverting would overwrite something else. Undo
    /// refuses rather than overwrites.
    Conflict(String),
    Database(sqlx::Error),
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "the tool can't undo its actions"),
            Self::Conflict(message) => write!(f, "cannot undo: {message}"),
            Self::Database(err) => write!(f, "cannot undo: {err}"),
        }
    }
}

impl std::error::Error for UndoError {}

impl From<sqlx::Error> for UndoError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
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

    /// Reverts one earlier call, given the data its handler passed to
    /// [`ToolContext::set_undo`], on `conn` (inside the undo's transaction). It must report
    /// [`UndoError::Conflict`] rather than overwrite a record that changed since. The
    /// default supports no undo.
    async fn undo(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError> {
        let _ = (conn, data);
        Err(UndoError::Unsupported)
    }
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
    /// An `external` call waits for the user's decision on this pending action; the
    /// handler didn't run. The calling step parks until the user decides.
    AwaitingApproval { pending_action_id: i64 },
    /// The user rejected this `external` call; the handler never runs.
    Rejected,
}

impl fmt::Display for ToolCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AwaitingApproval { pending_action_id } => write!(
                f,
                "waiting for the user's approval (pending action {pending_action_id})"
            ),
            Self::Rejected => write!(f, "the user rejected this action"),
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

    /// Whether `input` deserializes into the typed input.
    fn fits_input(&self, input: &Value) -> bool;

    async fn undo_json(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError>;
}

#[rocket::async_trait]
impl<T: Tool> ErasedTool for T {
    fn fits_input(&self, input: &Value) -> bool {
        T::Input::deserialize(input).is_ok()
    }

    async fn undo_json(&self, conn: &mut PgConnection, data: Value) -> Result<(), UndoError> {
        self.undo(conn, data).await
    }

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
            undo: None,
        };
        // The pending action this call runs the approved handler for, if any.
        let mut executes = None;
        let finished = match entry {
            None => Finished {
                status: "unknown_tool",
                output: None,
                result: Err(ToolCallError::UnknownTool),
            },
            Some(entry) if entry.info.permission == Permission::External => {
                match gate(pool, step_id, id, entry, &input).await {
                    Gate::Run { action_id, input } => {
                        executes = Some(action_id);
                        run(entry, &mut ctx, input).await
                    }
                    Gate::Done(finished) => finished,
                }
            }
            Some(entry) => run(entry, &mut ctx, input).await,
        };
        let spend = ctx.meter.spend;
        let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let error = finished
            .result
            .as_ref()
            .err()
            .map(|err| head(&err.to_string(), MAX_ERROR_BYTES).to_owned());
        let succeeded = finished.status == "succeeded";
        let record = Record {
            id,
            step_id,
            tool: name,
            // Only a `write` call that succeeded is an action to log.
            action: entry
                .filter(|entry| succeeded && entry.info.permission == Permission::Write)
                .map(|_| ctx.undo),
            executed: executes.filter(|_| succeeded),
        };

        match record
            .write(pool, &finished, error, spend, duration_ms)
            .await
        {
            Ok(()) => (spend, Some(id), finished.result),
            Err(err) => (spend, Some(id), Err(ToolCallError::Database(err))),
        }
    }

    /// Checks `input` against tool `name`'s input schema and typed input, as an invocation
    /// does before running the handler. Fails with `UnknownTool` or `InvalidInput`, whose
    /// message names the path and never quotes a value.
    pub fn check_input(&self, name: &str, input: &Value) -> Result<(), ToolCallError> {
        let entry = self.tools.get(name).ok_or(ToolCallError::UnknownTool)?;
        check(entry, input)
    }

    /// Runs tool `name`'s [`Tool::undo`] with `data` on `conn`. An unknown name is
    /// [`UndoError::Unsupported`].
    pub async fn undo(
        &self,
        name: &str,
        conn: &mut PgConnection,
        data: Value,
    ) -> Result<(), UndoError> {
        match self.tools.get(name) {
            Some(entry) => entry.tool.undo_json(conn, data).await,
            None => Err(UndoError::Unsupported),
        }
    }
}

/// The final write of one invocation: its `tool_calls` row, plus the `agent_actions` row of
/// a succeeded `write` call and the pending action a succeeded approved call executed, all
/// in one transaction.
struct Record<'a> {
    id: i64,
    step_id: i64,
    tool: &'a str,
    /// `Some(undo_data)` when the call is an action to log.
    action: Option<Option<Value>>,
    /// The pending action to mark `executed`.
    executed: Option<i64>,
}

impl Record<'_> {
    async fn write(
        self,
        pool: &PgPool,
        finished: &Finished,
        error: Option<String>,
        spend: Spend,
        duration_ms: i64,
    ) -> Result<(), sqlx::Error> {
        let mut tx = pool.begin().await?;
        sqlx::query(
            "UPDATE tool_calls SET status = $2, output = $3, error = $4, input_tokens = $5, \
             output_tokens = $6, cost_usd = $7, duration_ms = $8, finished_at = now() \
             WHERE id = $1",
        )
        .bind(self.id)
        .bind(finished.status)
        .bind(&finished.output)
        .bind(error)
        .bind(spend.input_tokens)
        .bind(spend.output_tokens)
        .bind(spend.cost_usd)
        .bind(duration_ms)
        .execute(&mut *tx)
        .await?;
        if let Some(undo_data) = self.action {
            if undo_data.is_none() {
                rocket::warn!(
                    "tools: write tool {} set no undo data (tool call {}); it can't be undone",
                    self.tool,
                    self.id
                );
            }
            sqlx::query(
                "INSERT INTO agent_actions (run_id, run_step_id, tool_call_id, tool, undo_data) \
                 SELECT run_id, id, $2, $3, $4 FROM run_steps WHERE id = $1",
            )
            .bind(self.step_id)
            .bind(self.id)
            .bind(self.tool)
            .bind(undo_data)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(action_id) = self.executed {
            sqlx::query(
                "UPDATE pending_actions SET status = 'executed', output = $2, \
                 executed_tool_call_id = $3, executed_at = now() \
                 WHERE id = $1 AND status = 'approved'",
            )
            .bind(action_id)
            .bind(&finished.output)
            .bind(self.id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
}

/// What the approval gate decided for an `external` call.
enum Gate {
    /// The user approved it: run the handler with `input`, then mark the action executed.
    Run { action_id: i64, input: Value },
    /// The call ends here without running the handler.
    Done(Finished),
}

/// The approval gate for an `external` call: checks the input, then finds or creates the
/// step's pending action for this tool and requested input and acts on its status.
async fn gate(
    pool: &PgPool,
    step_id: i64,
    tool_call_id: i64,
    entry: &Entry,
    input: &Value,
) -> Gate {
    if let Err(err) = check(entry, input) {
        return Gate::Done(Finished {
            status: "invalid_input",
            output: None,
            result: Err(err),
        });
    }
    let found = pending_action(pool, step_id, tool_call_id, entry.info.name, input).await;
    let (action_id, status, approved_input, output) = match found {
        Ok(found) => found,
        Err(err) => {
            return Gate::Done(Finished {
                status: "failed",
                output: None,
                result: Err(ToolCallError::Database(err)),
            });
        }
    };
    Gate::Done(match status.as_str() {
        "approved" => {
            return Gate::Run {
                action_id,
                input: approved_input.unwrap_or_else(|| input.clone()),
            };
        }
        // Steps are at-least-once: a rerun gets the stored output, not a second send.
        "executed" => {
            let output = output.unwrap_or(Value::Null);
            Finished {
                status: "succeeded",
                output: Some(output.clone()),
                result: Ok(output),
            }
        }
        "rejected" => Finished {
            status: "failed",
            output: None,
            result: Err(ToolCallError::Rejected),
        },
        _ => Finished {
            status: "awaiting_approval",
            output: None,
            result: Err(ToolCallError::AwaitingApproval {
                pending_action_id: action_id,
            }),
        },
    })
}

/// `(id, status, approved_input, output)` of the pending action keyed on (step, tool,
/// requested input), inserted as `pending` by `tool_call_id` when there is none yet.
async fn pending_action(
    pool: &PgPool,
    step_id: i64,
    tool_call_id: i64,
    tool: &str,
    input: &Value,
) -> Result<(i64, String, Option<Value>, Option<Value>), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pending_actions (run_id, run_step_id, tool_call_id, tool, input) \
         SELECT run_id, id, $2, $3, $4 FROM run_steps WHERE id = $1 \
         ON CONFLICT DO NOTHING",
    )
    .bind(step_id)
    .bind(tool_call_id)
    .bind(tool)
    .bind(input)
    .execute(pool)
    .await?;
    sqlx::query_as(
        "SELECT id, status, approved_input, output FROM pending_actions \
         WHERE run_step_id = $1 AND tool = $2 AND md5(input::text) = md5($3::jsonb::text)",
    )
    .bind(step_id)
    .bind(tool)
    .bind(input)
    .fetch_one(pool)
    .await
}

/// Checks `input` against the entry's input schema, then its typed input.
fn check(entry: &Entry, input: &Value) -> Result<(), ToolCallError> {
    schema::validate(&entry.info.input_schema, input).map_err(ToolCallError::InvalidInput)?;
    if entry.tool.fits_input(input) {
        Ok(())
    } else {
        // serde's message can quote the input, so only say that it didn't fit.
        Err(ToolCallError::InvalidInput(
            "$: does not fit the tool's input type".into(),
        ))
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
