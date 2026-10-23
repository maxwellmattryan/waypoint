# Agent runtime

Every agent workflow in Waypoint runs as a **run**: a fixed sequence of steps executed by the job queue, with each step's input, output, status, tokens and cost stored as it goes. Steps call the model through Waypoint's `claude -p` runner and call **tools**, typed Rust handlers with JSON schemas and a permission level. This page covers the shared machinery in `api/src/runs`, `api/src/tools` and `api/src/mcp`, the guardrails that enforce each tool's permission level ([approval gate](#approval-gate), [action log and undo](#action-log-and-undo), [review items](#review-items)), and the workflows the app ships in `api/src/workflows`: the [`tidy_application` sample](#sample-workflow-tidy_application) and the [memory writer](memory-writer.md). The app's **Review** page (`/review`) is where the user approves, edits or rejects pending actions, dismisses review items and undoes logged actions. Per-step tool allowlists build on this later.

## Tables

| Table | One row per | Holds |
|---|---|---|
| `agent_runs` | run of a workflow | `workflow`, `status` (`queued`, `running`, `paused`, `succeeded`, `failed`, `awaiting_approval`, `rejected`), `input`, `output`, `error`, token and cost totals, `job_id` (the latest `agent_run` job), `started_at`, `finished_at` |
| `run_steps` | step of a run | `run_id`, `ordinal` (0, 1, …; unique per run), `name`, `status` (`pending`, `running`, `succeeded`, `failed`, `awaiting_approval`), `attempts`, `input`, `output`, `error`, token and cost totals, `claude_call_id` |
| `tool_calls` | tool invocation in a step | `run_step_id`, `tool`, `permission`, `status` (`running`, `succeeded`, `failed`, `invalid_input`, `invalid_output`, `unknown_tool`, `awaiting_approval`), `input`, `output`, `error`, tokens, cost, `claude_call_id`, `duration_ms` |
| `pending_actions` | distinct `external` request in a step | `run_id`, `run_step_id`, `tool_call_id` (the call that asked), `tool`, `input` (as requested), `approved_input`, `status` (`pending`, `approved`, `rejected`, `executing`, `executed`, `failed`), `output`, `executed_tool_call_id` (the call that claimed it), `created_at`, `decided_at`, `executed_at`. Unique on (`run_step_id`, `tool`, `md5(input::text)`) |
| `agent_actions` | succeeded `write` tool call | `run_id`, `run_step_id`, `tool_call_id` (unique), `tool`, `undo_data`, `created_at`, `undone_at` |
| `review_items` | result a step wasn't sure enough to apply | `run_id`, `run_step_id`, `kind`, `confidence` (0..=1), `payload`, `status` (`pending`, `accepted`, `dismissed`), `created_at`, `resolved_at`. Unique on (`run_step_id`, `kind`, `md5(payload::text)`) |
| `memory_signals` | user decision on something an agent drafted ([learning signals](#learning-signals)) | `kind` (`approval`, `rejection`, `dismissal`), `run_id`, `pending_action_id` (unique; approvals and rejections), `review_item_id` (unique; dismissals), `subject` (the tool or the item's kind), `drafted`, `approved` and `changes` (approvals only), `reason` (dismissals only, 1..=1000 characters or `NULL`), `created_at`, `processed_at` (set once the memory writer has learned from it), `writer_run_id` (FK `agent_runs`, `SET NULL`; the writer run that claimed it). No foreign keys to the run, action or item it came from. Which memories a signal is evidence for lives in `memory_evidence` ([memory writer storage](memory-writer.md#storage)) |
| `run_memories` | memory a step [retrieved](#retrieval) | `run_id`, `run_step_id`, `memory_id` (cleared when the memory is deleted), a snapshot of the memory's `kind`, `topic` and `content` at retrieval time, `score`, `created_at`. Unique on (`run_step_id`, `memory_id`) |

Deleting a run deletes its steps, their tool calls, and its pending actions, actions, review items and run memories, but not its learning signals. The `claude_call_id` columns point at `claude_calls`; runs reach their model calls through their steps.

## Starting and running

`runs::start(pool, bus, wake, runtime, workflow, input)` stores the run (`queued`, or `paused` while the [kill switch](#kill-switch) is on), one `pending` step per workflow step in order, and one `agent_run` queue job with the payload `{"run_id": <id>}`, all in one transaction. An unknown workflow name is an error and writes nothing.

The worker runs the steps in order. The first step gets the run's input, each later step gets the previous step's output, and the run's output is the last step's output. One job runs the whole run, so the queue's concurrency limit counts runs, not steps.

Each step is recorded as it runs: `running` with `attempts + 1` and its input, then `succeeded` with its output, or `failed` with its error.

### Resume

When a run's job runs again, steps that already `succeeded` are skipped and their stored output feeds the next step. The first step that hasn't succeeded (`pending`, `failed`, or `running` because a crash orphaned it) runs again. A job runs again after:

- a queue retry,
- a usage-limit deferral,
- a requeue of jobs left `running` when the API stopped,
- the kill switch turning off, for a run it paused,
- `runs::resume(pool, bus, wake, run_id)`, which queues a new job for a `failed` run (the run is `paused` instead of `queued` while the kill switch is on). It returns `None` and queues nothing for any other status or an unknown id, so a run never has two jobs in flight.

> **Steps are at-least-once.** A step that dies after its side effects but before its `succeeded` write runs again. Step handlers, and the tools they call, must be idempotent. Nothing enforces this.

### Run status

| Step result | Job outcome | Run status | Run `error` | `finished_at` |
|---|---|---|---|---|
| every step succeeded | done | `succeeded` | cleared | set |
| retryable error, attempts left | retry | `queued` | step error | unchanged |
| retryable error, no attempts left | fail | `failed` | step error | set |
| usage limit | defer (the queue pauses) | `queued` | step error | unchanged |
| kill switch on before the run or a step starts | defer with no delay (the queue runs on) | `paused` | unchanged | unchanged |
| kill switch turned off | — (the job is claimed again) | `queued` | unchanged | unchanged |
| permanent error, or the workflow isn't registered | fail | `failed` | error | set |
| a tool call waits for approval ([parked](#parking)) | done | `awaiting_approval` | cleared | unchanged |
| the user rejects a pending action (not a job) | — | `rejected` | `the user rejected pending action <id>` | set |
| run already `succeeded`, `awaiting_approval` or `rejected` when the job starts | done (no-op) | unchanged | — | — |
| run row missing | fail | — | — | — |

A step returns `StepError::Retry`, `UsageLimit` or `Fail`. Model-call errors convert with `?`: a usage limit gives `UsageLimit` and any other runner error gives `Retry`, the same mapping the other job kinds use. Tool errors convert too: an unknown tool, an invalid input or output, or a rejected action gives `Fail`, and a failed handler, a database error or a call awaiting approval gives `Retry` (a step with a call awaiting approval parks whatever it returns, so that one is only a fallback). A database error inside the executor gives `Retry`.

## Kill switch

One global switch, stored in the single `kill_switch` row, stops every agent at once. It survives a restart, and a fresh database starts with it off. The dashboard's Agents card flips it; the API is `GET /kill-switch` (`{"paused": bool, "updated_at": …}`) and `PUT /kill-switch` with `{"paused": true|false}`. `PUT` takes the same local-`Host` and cross-site guards as every other write.

Turning it **on** (`kill_switch::set_paused(pool, bus, wake, true)`):

- every `queued` run becomes `paused`, in the same transaction as the switch,
- a `running` run finishes the step it's on, then becomes `paused` before its next step starts. That step stays `pending` and the run's `error` doesn't change. Its job goes back to the queue with its attempt given back (`job.deferred`), without pausing the queue,
- the worker claims no `agent_run` job. Other job kinds (`test_prompt`, `capture_extract`) still run, because they aren't agents,
- `runs::start` and `runs::resume` still store the run and its job, with the run `paused` instead of `queued`. They read the switch `FOR SHARE`, so a concurrent change waits for them and then moves their run too.

A run that is `awaiting_approval` keeps that status while the switch is on, and its pending actions can still be approved or rejected. Rejecting ends it `rejected` as usual. Approving its last pending action stores it `paused` with its job, so the parked step doesn't run until the switch is off.

Turning it **off** makes every `paused` run `queued` again and wakes the worker. Runs carry on from their first unfinished step; steps that already succeeded don't run again.

Setting the value it already has changes nothing and publishes nothing. Each change publishes, after the commit, `run.updated` for every run it moved and then `kill_switch.updated` with the new state. A step that is already running isn't interrupted: there is no way to cancel a step mid-flight.

[`DELETE /data`](google-oauth.md#delete-all-data) turns the switch on first and leaves it on, so nothing starts on the emptied database until you resume agents.

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
4. for an `external` tool, passes the [approval gate](#approval-gate), which may end the call without running the handler,
5. runs the handler; an error ends `failed`,
6. checks the output against the output schema; a mismatch ends `invalid_output`, with the output stored for debugging,
7. otherwise ends `succeeded` with the output. A succeeded `write` call also gets its [`agent_actions` row](#action-log-and-undo), written in the same transaction as the final `tool_calls` update.

Each row records the input, output, error, duration, tokens and cost, and `claude_call_id`: the model call that asked for the tool (empty for `ctx.tool` and for agent calls that failed). Validation errors name the JSON path and the expected type, such as `$.items[2].title: expected string`, and never quote the value.

### Permission levels

| Level | Meaning |
|---|---|
| `read` | Reads data; no side effects. |
| `write` | Changes Waypoint's own data; reversible and logged with undo. |
| `external` | Leaves Waypoint (sends, submits, calls out); always passes an approval gate. |

Levels are stored on every tool call and enforced in the registry, so step code (`ctx.tool`) and the model (through the [MCP server](#mcp-server)) get the same rules: `external` calls stop at the [approval gate](#approval-gate), and `write` calls are logged for [undo](#action-log-and-undo).

## Approval gate

An `external` call never runs its handler on first request. Once its input passes the schema check (a bad input still ends `invalid_input` and creates nothing), the registry finds or creates the step's `pending_actions` row for that tool and requested input, and acts on its status:

| Pending action | What the call does | `tool_calls` row |
|---|---|---|
| new, or `pending` | returns `ToolCallError::AwaitingApproval { pending_action_id }`; the step [parks](#parking) | `awaiting_approval` |
| `approved` | claims the action (below), then runs the handler with `approved_input`. In the same transaction as the row, the action becomes `executed` with `output` and `executed_at` when the call succeeded, `failed` otherwise | as for any call |
| `executing` | another attempt claimed it and is running the handler: returns `ToolCallError::Executing`, which retries the step, so a later attempt gets the stored output | `failed` |
| `executed` | returns the stored `output` without running the handler again (steps are at-least-once) | `succeeded` with that output |
| `failed` | returns `ToolCallError::ExecutionFailed`, which fails the step; the handler never runs a second time | `failed` |
| `rejected` | returns `ToolCallError::Rejected`, which fails the step | `failed`, `the user rejected this action` |

The row is keyed on (step, tool, requested input), so the step repeating the same call after approval finds it again. A call with a different input (a model may ask differently on resume) creates a new pending action.

An approved action runs at most once. Before its handler runs, the call claims it with `UPDATE pending_actions SET status = 'executing', executed_tool_call_id = … WHERE id = … AND status = 'approved'`, so of two concurrent attempts of the same step (a job whose lease expired while the first attempt was still running, say) only the one that wins runs the handler, and the other acts on the status it finds. An external call can't be taken back, so a claimed action is never re-run: a handler failure or invalid output marks it `failed` and fails the step, and an action left `executing` by a crashed process stays there, so its step retries until it runs out of attempts and fails. Either way the user starts a new run to try again.

### Parking

When a step made at least one call that is awaiting approval, it parks whatever it returned (`Ok`, any `Err`, or an error it swallowed): in one transaction the step becomes `awaiting_approval` with its spend added and no output, and the run becomes `awaiting_approval` with `error` cleared and `finished_at` unchanged. The job ends `done` with `{"run_id": <id>, "status": "awaiting_approval"}`. A job that starts on an `awaiting_approval` or `rejected` run does nothing.

### Deciding

- `POST /pending-actions/{id}/approve` with `{}` approves the requested input; `{"input": {…}}` approves an edited one, checked against the tool's input schema and typed input (422 `{"errors":[{"field":"input","message":"<path>: <expected>"}]}` otherwise, never quoting values). The action becomes `approved` with `approved_input` and `decided_at`. When the run has no other `pending` action, it becomes `queued` (`paused` while the [kill switch](#kill-switch) is on, read `FOR SHARE` like `runs::start`) with a new `agent_run` job in the same transaction and the worker is woken; the parked step then runs again (the steps before it are skipped as usual). While other actions are still pending, the run stays `awaiting_approval`.
- `POST /pending-actions/{id}/reject` rejects the action and every other `pending` action of the run, fails the run's `awaiting_approval` steps, and ends the run `rejected` with the error `the user rejected pending action <id>` and `finished_at`, in one transaction. The handler never runs. `runs::resume` only queues `failed` runs, so a rejected run is final.

Both answer 409 with nothing changed unless the action is `pending` and its run `awaiting_approval`; that includes a run that is still `running` because its step hasn't parked yet. Approve is also 409 when the tool is no longer registered. Both lock the pending action, then its run, so they serialize with each other and with parking. `runs::approvals::{approve, reject}` are the same operations as library functions.

Each decision records a [learning signal](#learning-signals) in its transaction: approve one `approval` (with the diff from the requested to the approved input), reject one `rejection` for the action the user rejected only, not the run's other pending actions it also rejects. A refused decision (404, 409, 422) records nothing.

## Action log and undo

A `write` tool's handler calls `ctx.set_undo(data)` with what its undo needs, and implements `Tool::undo`:

```rust
async fn undo(&self, conn: &mut sqlx::PgConnection, data: Value) -> Result<(), UndoError>;
```

The default returns `UndoError::Unsupported`. `UndoError` is `Unsupported`, `Conflict(String)` or `Database(sqlx::Error)`.

Every succeeded `write` call inserts one `agent_actions` row with the `undo_data` in the same transaction as the call's final `tool_calls` update. A write tool that never called `set_undo` still succeeds; its row has `undo_data = NULL`, a warning is logged, and it can't be undone. The handler's own write goes through the pool, so it isn't atomic with the log row (a known gap).

`POST /actions/{id}/undo` reverts exactly one action in one transaction: it locks the row, runs the tool's `undo` on the transaction's connection with `undo_data`, and sets `undone_at`. It is 409 with nothing changed when the action is already undone, has no `undo_data`, its tool isn't registered, or the undo reports a conflict. The conflict policy is **refuse**: an undo whose record changed or is gone reports `Conflict` rather than overwrite anything. Undo publishes no event, since the run doesn't change.

## Review items

`ctx.review_if_unsure(kind, confidence, payload)` sends a result the step isn't sure of to the review queue. When `confidence` is below `runs::REVIEW_THRESHOLD` (0.8) or isn't finite, it stores a `pending` `review_items` row and returns `Some(id)`, and the step must not apply the result. Otherwise it returns `None`. The confidence is stored clamped to 0..=1, with NaN stored as 0, and a rerun of the step gets the same item back for the same kind and payload. `GET /review-items` lists the pending ones. `POST /review-items/{id}/dismiss` with `{}` or `{"reason": "…"}` resolves one without applying it: in one transaction the item becomes `dismissed` with `resolved_at` set, by a statement that only matches a `pending` item, and a `dismissal` [learning signal](#learning-signals) is recorded with the payload and the reason. A second dismiss is 409 with nothing changed. `reason` is trimmed, a blank or absent one is stored as `NULL`, and one over 1000 characters is 422 `{"errors":[{"field":"reason","message":"At most 1000 characters"}]}`. The reason is never logged. Accepting an item (applying its result) is per workflow and comes later.

## Learning signals

`memory_signals` (`api/src/memory_signals.rs`) is the raw evidence the [memory writer](memory-writer.md) learns from: one row per approval, rejection or dismissal, written in the same transaction as the decision, so a stored decision always has its signal and a refused one has none. That transaction also schedules a `memory_writer` run 60 s out, unless one is already waiting, so a burst of decisions shares one model call; the run claims the unprocessed signals and marks them processed. The unique `pending_action_id` and `review_item_id` make one signal per decision a database fact, and a CHECK keeps each kind's columns consistent. There are no foreign keys to the run, action or item a signal came from (the only one is the writer's claim, `writer_run_id`), so a signal outlives them. Nothing over HTTP deletes a signal; only `DELETE /data` empties the table.

An approval's `changes` is the diff from `drafted` (the requested input) to `approved` (`approved_input`), `[]` when the user didn't edit it. Each entry is `{op, path, before, after}`: `path` is an RFC 6901 JSON Pointer (`/message`, `/a~1b`, `""` for the root), and `op` is `add` (`before` is `null`), `remove` (`after` is `null`) or `replace`. Objects are compared key by key, recursively, in lexicographic key order, and the entries come out in that order; every other value (arrays, scalars, a change of type) is compared and replaced whole. An explicit `null` counts as present.

`GET /memory-signals` lists them read-only; nothing over HTTP writes, edits or deletes one.

## Retrieval

A step pulls what Waypoint knows about the user with `ctx.memories(&retrieval::Query { text, tags, kinds, limit })` (`api/src/retrieval.rs`). It searches the `memories` table and records what it found on the step. There is no HTTP route for it.

- **Confirmed only.** `proposed` memories are never returned, whatever they match.
- **OR over words.** The text is split into runs of letters and digits, lowercased, deduplicated (at most 64 words, `or` dropped) and joined with `or` for `websearch_to_tsquery('english', …)`, so no search operator from the text reaches Postgres. A memory matches when it shares at least one stemmed, non-stop-word term with the text, in its content, topic or tags (the same `search` column the memories page uses). AND over every word, `websearch_to_tsquery`'s default, would almost never match a sentence.
- **Order.** Best match first (`ts_rank`, returned as `score`), then newest `updated_at`, then `id` descending.
- **Filters.** `tags` (normalised like stored tags) keeps memories carrying at least one of them; `kinds` keeps memories of one of those kinds. Filters only narrow what the text matched.
- **No usable terms.** Text that is blank, punctuation or stop words only returns the confirmed memories carrying any of the tags, newest `updated_at` first, with score 0. With no tags either it returns nothing, and blank text with no tags runs no query.
- **Limit.** 5 by default, clamped to 1..=20.

**Recording.** `ctx.memories` stores one `run_memories` row per returned memory, in rank order, with a snapshot of its kind, topic and content and its score, so the run inspector shows what the step actually saw after the memory is edited or deleted. Steps are at-least-once, so the rows are a union across attempts: a rerun (retry, resume, after approval) that retrieves the same memory adds no second row, and a memory first retrieved on a later attempt is added. When it adds a row it publishes `run.updated` (best effort). Deleting the memory keeps the row with `memory_id` cleared; deleting the run deletes the rows.

**Prompt block.** `retrieval::context(&found)` renders retrieved memories as the text a model-calling step puts in its `ClaudeRequest` prompt: a heading line, then `- [memory <id> · <kind> · <topic>] <content>` per memory (newlines in content become spaces), or `""` for none. `tidy_application` makes no model call, so its `draft` step renders no prompt block; it retrieves and records, then appends the confirmed `answer` and `preference` memories it found to the message it drafts (see the [sample workflow](#sample-workflow-tidy_application)). The first model-calling workflow uses the block.

Embeddings (pgvector) are deferred until the recall cases in the [`tidy_application` eval set](evals.md#the-tidy_application-eval-set) show full-text search missing relevant memories, for example through synonyms.

### Supported schema subset

Tool schemas are checked by a small in-repo checker (`api/src/tools/schema.rs`), not a JSON Schema crate. Supported keywords:

- `type`: one of `object`, `array`, `string`, `integer`, `number`, `boolean`, `null`, or a non-empty array of them (for example `["string", "null"]`),
- `properties`, `required`, `additionalProperties` (`false` or `true` only), `items`, `enum`,
- `description` and `title` (ignored when validating).

Any other keyword (`oneOf`, `$ref`, `minimum`, `pattern`, …) makes registration fail.

## MCP server

A step hands the model Waypoint's tools with `ctx.call_with_tools(req)` or `ctx.call_structured_with_tools::<T>(req, schema)`. They work like `call`/`call_structured`, except that every registered tool is available to the model through Waypoint's MCP server, and no other tool is. `external` tools pass the [approval gate](#approval-gate) as they do for step code, so a model asking for one parks the step even when the step's code returns `Ok`. Plain `call`/`call_structured` never pass any MCP flags.

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
| `tools/list` | `{"tools": [{name, description, inputSchema}]}` for every registered tool, by name. No `outputSchema`. |
| `tools/call` `{name, arguments}` | runs the tool through the registry (missing `arguments` counts as `{}`) |

Notifications (no `id`) get no response. Errors: an unknown method is `-32601`, a line that isn't JSON is `-32700`, and a batch array or any other non-object is `-32600`.

How a `tools/call` ends:

| Registry result | Response | `tool_calls` row |
|---|---|---|
| output | `{"content":[{"type":"text","text":"<output as compact JSON>"}],"isError":false}` | as for `ctx.tool` |
| unknown tool | JSON-RPC error `-32602`, `no tool has this name` | `unknown_tool` |
| invalid input or output, handler or model-call failure, rejected action | `{"content":[{"type":"text","text":"<error>"}],"isError":true}`, so the model can react | as for `ctx.tool` |
| database error | `isError` result `the tool call could not be recorded`; the sqlx error goes to the log only | none or `running` |
| awaiting approval (`external` tool) | `isError` result `this action is waiting for the user's approval; end your turn without retrying it`; the handler doesn't run and the step parks | `awaiting_approval`, plus a `pending` pending action |
| `name` missing or not a string | JSON-RPC error `-32602`, `invalid params` | none |

Error texts never quote input values.

### Linking and spend

Tool calls made during an agent call are recorded on the calling step. When the outer call succeeds, every `tool_calls` row its session created gets `claude_call_id` set to the outer call's `claude_calls.id`; when it fails, they keep `claude_call_id` empty. The model calls a tool handler makes count against the step (and so the run) even when the outer call fails. A usage limit inside a tool's own model call reaches the model as an `isError` result, not as the step's outcome.

## Sample workflow: `tidy_application`

The production runtime (`workflows::runtime()`) registers two workflows: `tidy_application`, below, and the [`memory_writer`](memory-writer.md), which has one step, `write`, and no tools. `tidy_application` exercises every guardrail without calling a model. Its run input is `{"application_id", "note", "message", "confidence"}`, and each step passes its input through and adds to it:

| Step | Tool (permission) | What it does |
|---|---|---|
| `inspect` | `get_application` (`read`) | Reads `{id, company, title, status}` of the application and adds it as `application`. An unknown id fails the tool (`application <id> doesn't exist`), which retries and then fails the run. |
| `note` | `add_note` (`write`) | `review_if_unsure("tidy.note", confidence, {application_id, note})`: below 0.8 it adds `review_item_id` and writes nothing. Otherwise `add_note` adds a `note` to the application's timeline, adds `note_event_id`, and logs an action whose undo deletes that note (a conflict when it is gone). |
| `draft` | `draft_follow_up` (`external`) | First [retrieves](#retrieval) the confirmed memories matching `"<company> <title> <message>"` (company and title from `application`) with the default limit, and records them on the step. Then it builds the tool's `message` from the run input's `message` and the confirmed `answer` and `preference` memories it retrieved, in retrieval order: each gives one line, its content trimmed with newlines turned into spaces, skipping blank and duplicate lines. With at least one line the message is `<message>\n\n<line>\n<line>…`; otherwise (nothing retrieved, or only `fact` memories, which are recorded but don't change the draft) it is the input `message` unchanged. Then always runs the tool with that message, so every run reaches the gate and the Review page shows the built message. After approval it returns `{"draft": "<message> (to <company>)"}`, which is the run's output. It sends and stores nothing: agents draft, the user sends. |

The three tools are also in the MCP server's `tools/list`. The README's "Try the sample workflow" walks through it with curl.

Its labelled eval set is `api/evals/tidy_application.json`, run with `cargo run --bin eval -- tidy_application` (see [evals.md](evals.md)).

## Routes

Every route answers only a local `Host` (403 otherwise) and answers 500 on a database error. The `POST` routes also refuse a cross-site request (`Sec-Fetch-Site: cross-site`) and an HTML form body with 403, and the ones with a JSON body answer 415 to any other content type and 400 to a body that doesn't parse or has unknown fields.

- `GET /runs?limit=&before=`: runs newest first (`id` descending). `limit` defaults to 50 and is clamped to 1..=200. `before` returns only runs with `id < before`, so the next page is `?before=<last id>`. With no runs the body is `[]`.
- `GET /runs/{id}`: the run with `steps` ordered by `ordinal`, each with its `tool_calls` ordered by `id`, and `memories`: every `run_memories` row of the run, by step `ordinal`, then `id`, each with `id`, `run_id`, `run_step_id`, `step` (the step's name), `memory_id` (`null` once deleted), `kind`, `topic`, `content` (the snapshot), `score` and `created_at`; `[]` when the run used none. The run inspector lists them under "Memories used", each linking to `/memories?memory=<id>`. An unknown id is 404.
- `GET /pending-actions`: up to 200 `pending` actions, oldest first.
- `POST /pending-actions/{id}/approve` (`{}` or `{"input": {…}}`) and `POST /pending-actions/{id}/reject` (no body): see [Deciding](#deciding). 200 with the `PendingAction`; 404 for an unknown id.
- `GET /actions?run_id=&limit=&before=`: the action log newest first, optionally for one run; `limit` and `before` work as on `GET /runs`.
- `POST /actions/{id}/undo` (no body): see [Action log and undo](#action-log-and-undo). 200 with the `AgentAction`; 404 for an unknown id.
- `GET /review-items`: up to 200 `pending` review items, oldest first.
- `POST /review-items/{id}/dismiss` (`{}` or `{"reason": "…"}`): see [Review items](#review-items). 200 with the `ReviewItem`; 409 when it isn't `pending`; 404 for an unknown id; 422 for a reason over 1000 characters.
- `GET /memory-signals?kind=&limit=&before=`: [learning signals](#learning-signals) newest first; `limit` and `before` work as on `GET /runs`. `kind` is `approval`, `rejection` or `dismissal`; blank means any, and an unknown one is 422 `{"errors":[{"field":"kind","message":"Unknown kind"}]}`.
- `POST /memories/learn` (no body): asks the [memory writer](memory-writer.md#scheduling) to learn now. 202 `{"run_id", "reused"}`: the waiting writer run (`reused: true`), pulled to start now when it's `queued`, or a new one that may start now.
- `POST /applications/{id}/tidy` with `{"note", "message", "confidence"?}` starts [`tidy_application`](#sample-workflow-tidy_application) and answers 201 with the queued `Run`. `note` and `message` are required, trimmed and at most 20,000 characters; `confidence` defaults to 1 and must be within 0..=1; failures are 422 naming the field. An unknown application is 404, checked before the run starts.

There are no generic routes to start, resume or cancel runs.

## Events

`GET /events` sends `run.updated` whenever a run changes, so the Runs pages stay live:

```json
{ "type": "run.updated", "run": { "id": 1, "workflow": "…", "status": "running", … } }
```

`run` is the `agent_runs` row exactly as `GET /runs` returns it, never the steps or tool calls. A client that wants those refetches `GET /runs/{id}` when it sees an event for that run.

It fires after each of these commits, never inside a transaction:

- `runs::start` and `runs::resume` (the run is `queued`, or `paused` while the kill switch is on; this one carries the row they return, with no extra read),
- the kill switch turning on or off, for each run it moved,
- the executor marking the run `paused` because the kill switch is on,
- the executor marking the run `running`,
- a step going `running`,
- each `StepContext::tool` call returning, whether the tool succeeded or failed,
- each MCP `tools/call` that reached the registry returning (a row was written or attempted; a call without a usable `name` publishes nothing),
- a step recording retrieved memories (`StepContext::memories` added at least one `run_memories` row),
- a step parking (the run is `awaiting_approval`),
- an approve that queues the run again, and a reject (the run is `rejected`),
- a step succeeding or failing (a failed step's event already has the run's settled status: `queued` for a retry or a usage-limit deferral, `failed` otherwise),
- the run succeeding, or failing because its workflow isn't in the runtime,
- the best-effort settle after a database error.

So a one-step run whose step makes one tool call sends `queued`, `running` (run), `running` (step), `running` (tool returned), `running` (step succeeded) and `succeeded`.

`runs::publish(pool, bus, run_id)` reads the row and publishes it. It is best effort: if the read fails it logs a warning and publishes nothing, and the run carries on. Like every event there is no replay; after `app.connected` or `app.lagged` a client refetches.
