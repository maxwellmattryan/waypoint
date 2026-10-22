# Agent runtime

Every agent workflow in Waypoint runs as a **run**: a fixed sequence of steps executed by the job queue, with each step's input, output, status, tokens and cost stored as it goes. Steps call the model through Waypoint's `claude -p` runner and call **tools**, typed Rust handlers with JSON schemas and a permission level. This page covers the shared machinery in `api/src/runs`, `api/src/tools` and `api/src/mcp`. No real workflows or tools are registered yet; the approval gate, undo, per-step tool allowlists and the run inspector UI build on this later.

## Tables

| Table | One row per | Holds |
|---|---|---|
| `agent_runs` | run of a workflow | `workflow`, `status` (`queued`, `running`, `succeeded`, `failed`), `input`, `output`, `error`, token and cost totals, `job_id` (the latest `agent_run` job), `started_at`, `finished_at` |
| `run_steps` | step of a run | `run_id`, `ordinal` (0, 1, …; unique per run), `name`, `status` (`pending`, `running`, `succeeded`, `failed`), `attempts`, `input`, `output`, `error`, token and cost totals, `claude_call_id` |
| `tool_calls` | tool invocation in a step | `run_step_id`, `tool`, `permission`, `status`, `input`, `output`, `error`, tokens, cost, `claude_call_id`, `duration_ms` |

Deleting a run deletes its steps and their tool calls. The `claude_call_id` columns point at `claude_calls`; runs reach their model calls through their steps.

## Starting and running

`runs::start(pool, bus, wake, runtime, workflow, input)` stores the run (`queued`), one `pending` step per workflow step in order, and one `agent_run` queue job with the payload `{"run_id": <id>}`, all in one transaction. An unknown workflow name is an error and writes nothing.

The worker runs the steps in order. The first step gets the run's input, each later step gets the previous step's output, and the run's output is the last step's output. One job runs the whole run, so the queue's concurrency limit counts runs, not steps.

Each step is recorded as it runs: `running` with `attempts + 1` and its input, then `succeeded` with its output, or `failed` with its error.

### Resume

When a run's job runs again, steps that already `succeeded` are skipped and their stored output feeds the next step. The first step that hasn't succeeded (`pending`, `failed`, or `running` because a crash orphaned it) runs again. A job runs again after:

- a queue retry,
- a usage-limit deferral,
- a requeue of jobs left `running` when the API stopped,
- `runs::resume(pool, bus, wake, run_id)`, which queues a new job for a `failed` run. It returns `None` and queues nothing for any other status or an unknown id, so a run never has two jobs in flight.

> **Steps are at-least-once.** A step that dies after its side effects but before its `succeeded` write runs again. Step handlers, and the tools they call, must be idempotent. Nothing enforces this.

### Run status

| Step result | Job outcome | Run status | Run `error` | `finished_at` |
|---|---|---|---|---|
| every step succeeded | done | `succeeded` | cleared | set |
| retryable error, attempts left | retry | `queued` | step error | unchanged |
| retryable error, no attempts left | fail | `failed` | step error | set |
| usage limit | defer (the queue pauses) | `queued` | step error | unchanged |
| permanent error, or the workflow isn't registered | fail | `failed` | error | set |
| run already `succeeded` when the job starts | done (no-op) | unchanged | — | — |
| run row missing | fail | — | — | — |

A step returns `StepError::Retry`, `UsageLimit` or `Fail`. Model-call errors convert with `?`: a usage limit gives `UsageLimit` and any other runner error gives `Retry`, the same mapping the other job kinds use. Tool errors convert too: an unknown tool or an invalid input or output gives `Fail`, and a failed handler or database error gives `Retry`. A database error inside the executor gives `Retry`.

## Tokens and cost

Steps make model calls through their `StepContext` (`call`, `call_structured`). Each successful call adds to the step:

- `input_tokens`: the call's `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`, missing values counted as 0,
- `output_tokens`: the call's `output_tokens`,
- `cost_usd`: the call's reported cost, missing counted as 0,

and sets the step's `claude_call_id` to that call's `claude_calls.id` (the most recent when a step makes several). A step's totals add up across attempts and include the spend of its tool calls, even ones that failed. Every step write adds the same amounts to the run in the same transaction, so a run's totals always equal the sum of its steps' totals.

Failed model calls return no log id or usage from the runner, so they aren't counted. Their rows in `claude_calls` still record what the CLI reported.

## Tools

A tool implements `tools::Tool`:

```rust
#[rocket::async_trait]
pub trait Tool: Send + Sync + 'static {
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    const PERMISSION: Permission;
    type Input: DeserializeOwned + Send;
    type Output: Serialize + Send;
    fn input_schema() -> Value;
    fn output_schema() -> Value;
    async fn call(&self, ctx: &mut ToolContext<'_>, input: Self::Input) -> Result<Self::Output, ToolError>;
}
```

`Input` types should use `#[serde(deny_unknown_fields)]`. `ToolContext` gives the pool and metered `call`/`call_structured`, whose spend is stored on the tool call and added to the calling step. Register tools with `Registry::register`, which refuses a duplicate name or a schema outside the supported subset, and pass the registry to `Runtime::new` with the workflows. `Registry::describe` lists the tools, which is what the [MCP server](#mcp-server) offers the model.

A step calls a tool directly with `ctx.tool(name, input)`, or lets the model call tools during `ctx.call_with_tools` (see [MCP server](#mcp-server)). Each invocation:

1. inserts a `tool_calls` row (`running`); if that fails, the handler never runs,
2. for an unknown name, ends `unknown_tool` with no permission, without calling a handler,
3. checks the input against the input schema and deserializes it into `Input`; either failing ends `invalid_input` without calling the handler,
4. runs the handler; an error ends `failed`,
5. checks the output against the output schema; a mismatch ends `invalid_output`, with the output stored for debugging,
6. otherwise ends `succeeded` with the output.

Each row records the input, output, error, duration, tokens and cost, and `claude_call_id`: the model call that asked for the tool (empty for `ctx.tool` and for agent calls that failed). Validation errors name the JSON path and the expected type, such as `$.items[2].title: expected string`, and never quote the value.

### Permission levels

| Level | Meaning |
|---|---|
| `read` | Reads data; no side effects. |
| `write` | Changes Waypoint's own data; reversible and logged with undo. |
| `external` | Leaves Waypoint (sends, submits, calls out); always passes an approval gate. |

Levels are stored on every tool call. Until the approval gate for `external` tools exists (a later job), the [MCP server](#mcp-server) neither lists nor runs them, so only step code can call one.

### Supported schema subset

Tool schemas are checked by a small in-repo checker (`api/src/tools/schema.rs`), not a JSON Schema crate. Supported keywords:

- `type`: one of `object`, `array`, `string`, `integer`, `number`, `boolean`, `null`, or a non-empty array of them (for example `["string", "null"]`),
- `properties`, `required`, `additionalProperties` (`false` or `true` only), `items`, `enum`,
- `description` and `title` (ignored when validating).

Any other keyword (`oneOf`, `$ref`, `minimum`, `pattern`, …) makes registration fail.

## MCP server

A step hands the model Waypoint's tools with `ctx.call_with_tools(req)` or `ctx.call_structured_with_tools::<T>(req, schema)`. They work like `call`/`call_structured`, except that every registered `read` and `write` tool is available to the model through Waypoint's MCP server, and no other tool is. Plain `call`/`call_structured` never pass any MCP flags.

### Transport

There is no HTTP endpoint. Each agent call gets its own session:

1. A temp dir (`waypoint-mcp-…`, mode 0700), separate from the CLI's empty working dir, holds a Unix socket `mcp.sock` that the API listens on and a generated `mcp.json`:
   `{"mcpServers":{"waypoint":{"type":"stdio","command":"<waypoint-mcp>","args":["--socket","<dir>/mcp.sock"]}}}`. It holds no secrets and no env.
2. `claude` is spawned with the usual isolated argv (`--tools ""`, `--strict-mcp-config`, never `--bare`), then `--model`/`--json-schema` when given, then `--mcp-config <dir>/mcp.json --allowedTools mcp__waypoint` last, because both flags are variadic.
3. The CLI starts the `waypoint-mcp` bin (`api/src/bin/waypoint-mcp.rs`), a relay that copies its stdin to the socket and the socket to its stdout. It reads no env and opens no database; the protocol is served inside the API process by `api/src/mcp`.
4. When the CLI exits (success, error, timeout or kill), the session stops accepting connections, lets tool calls already in flight finish and write their rows, then removes the dir.

The relay is found at `WAYPOINT_MCP_BIN`, or `waypoint-mcp` beside the API binary by default; `cargo build -p waypoint-api --bins` builds it, and `scripts/dev.sh` does so before starting the API. If the relay is missing, or the dir or socket can't be set up, the call fails with `ClaudeError::Process` naming the cause before `claude` is spawned, and the step retries.

### Protocol

Newline-delimited JSON-RPC 2.0 over the relay's stdio. Requests on one connection are handled one at a time in arrival order, and a line over 4 MiB closes the connection.

| Method | Result |
|---|---|
| `initialize` | `protocolVersion` (the client's when it is `2025-06-18`, `2025-03-26` or `2024-11-05`, otherwise `2025-06-18`), `capabilities: {tools: {listChanged: false}}`, `serverInfo: {name: "waypoint", version}` |
| `ping` | `{}` |
| `tools/list` | `{"tools": [{name, description, inputSchema}]}` for every registered tool except `external` ones, by name. No `outputSchema`. |
| `tools/call` `{name, arguments}` | runs the tool through the registry (missing `arguments` counts as `{}`) |

Notifications (no `id`) get no response. Errors: an unknown method is `-32601`, a line that isn't JSON is `-32700`, and a batch array or any other non-object is `-32600`.

How a `tools/call` ends:

| Registry result | Response | `tool_calls` row |
|---|---|---|
| output | `{"content":[{"type":"text","text":"<output as compact JSON>"}],"isError":false}` | as for `ctx.tool` |
| unknown tool | JSON-RPC error `-32602`, `no tool has this name` | `unknown_tool` |
| invalid input or output, handler or model-call failure | `{"content":[{"type":"text","text":"<error>"}],"isError":true}`, so the model can react | as for `ctx.tool` |
| database error | `isError` result `the tool call could not be recorded`; the sqlx error goes to the log only | none or `running` |
| `external` tool | `isError` result `this tool needs the user's approval and isn't available to agents yet`; the handler never runs | none |
| `name` missing or not a string | JSON-RPC error `-32602`, `invalid params` | none |

Error texts never quote input values.

### Linking and spend

Tool calls made during an agent call are recorded on the calling step. When the outer call succeeds, every `tool_calls` row its session created gets `claude_call_id` set to the outer call's `claude_calls.id`; when it fails, they keep `claude_call_id` empty. The model calls a tool handler makes count against the step (and so the run) even when the outer call fails. A usage limit inside a tool's own model call reaches the model as an `isError` result, not as the step's outcome.

## Routes

Both routes are read-only, answer only a local `Host` (403 otherwise) and answer 500 on a database error.

- `GET /runs?limit=&before=`: runs newest first (`id` descending). `limit` defaults to 50 and is clamped to 1..=200. `before` returns only runs with `id < before`, so the next page is `?before=<last id>`. With no runs the body is `[]`.
- `GET /runs/{id}`: the run with `steps` ordered by `ordinal`, each with its `tool_calls` ordered by `id`. An unknown id is 404.

There are no routes to start, resume or cancel runs yet.
