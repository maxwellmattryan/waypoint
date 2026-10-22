# Agent runtime

Every agent workflow in Waypoint runs as a **run**: a fixed sequence of steps executed by the job queue, with each step's input, output, status, tokens and cost stored as it goes. Steps call the model through Waypoint's `claude -p` runner and call **tools**, typed Rust handlers with JSON schemas and a permission level. This page covers the shared machinery in `api/src/runs` and `api/src/tools`. No real workflows or tools are registered yet; the MCP server, the approval gate, undo and the run inspector UI build on this later.

## Tables

| Table | One row per | Holds |
|---|---|---|
| `agent_runs` | run of a workflow | `workflow`, `status` (`queued`, `running`, `paused`, `succeeded`, `failed`), `input`, `output`, `error`, token and cost totals, `job_id` (the latest `agent_run` job), `started_at`, `finished_at` |
| `run_steps` | step of a run | `run_id`, `ordinal` (0, 1, …; unique per run), `name`, `status` (`pending`, `running`, `succeeded`, `failed`), `attempts`, `input`, `output`, `error`, token and cost totals, `claude_call_id` |
| `tool_calls` | tool invocation in a step | `run_step_id`, `tool`, `permission`, `status`, `input`, `output`, `error`, tokens, cost, `claude_call_id`, `duration_ms` |

Deleting a run deletes its steps and their tool calls. The `claude_call_id` columns point at `claude_calls`; runs reach their model calls through their steps.

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
| run already `succeeded` when the job starts | done (no-op) | unchanged | — | — |
| run row missing | fail | — | — | — |

A step returns `StepError::Retry`, `UsageLimit` or `Fail`. Model-call errors convert with `?`: a usage limit gives `UsageLimit` and any other runner error gives `Retry`, the same mapping the other job kinds use. Tool errors convert too: an unknown tool or an invalid input or output gives `Fail`, and a failed handler or database error gives `Retry`. A database error inside the executor gives `Retry`.

## Kill switch

One global switch, stored in the single `kill_switch` row, stops every agent at once. It survives a restart, and a fresh database starts with it off. The dashboard's Agents card flips it; the API is `GET /kill-switch` (`{"paused": bool, "updated_at": …}`) and `PUT /kill-switch` with `{"paused": true|false}`. `PUT` takes the same local-`Host` and cross-site guards as every other write.

Turning it **on** (`kill_switch::set_paused(pool, bus, wake, true)`):

- every `queued` run becomes `paused`, in the same transaction as the switch,
- a `running` run finishes the step it's on, then becomes `paused` before its next step starts. That step stays `pending` and the run's `error` doesn't change. Its job goes back to the queue with its attempt given back (`job.deferred`), without pausing the queue,
- the worker claims no `agent_run` job. Other job kinds (`test_prompt`, `capture_extract`) still run, because they aren't agents,
- `runs::start` and `runs::resume` still store the run and its job, with the run `paused` instead of `queued`. They read the switch `FOR SHARE`, so a concurrent change waits for them and then moves their run too.

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

`Input` types should use `#[serde(deny_unknown_fields)]`. `ToolContext` gives the pool and metered `call`/`call_structured`, whose spend is stored on the tool call and added to the calling step. Register tools with `Registry::register`, which refuses a duplicate name or a schema outside the supported subset, and pass the registry to `Runtime::new` with the workflows. `Registry::describe` lists the tools for the later MCP server.

A step calls a tool with `ctx.tool(name, input)`. Each invocation:

1. inserts a `tool_calls` row (`running`); if that fails, the handler never runs,
2. for an unknown name, ends `unknown_tool` with no permission, without calling a handler,
3. checks the input against the input schema and deserializes it into `Input`; either failing ends `invalid_input` without calling the handler,
4. runs the handler; an error ends `failed`,
5. checks the output against the output schema; a mismatch ends `invalid_output`, with the output stored for debugging,
6. otherwise ends `succeeded` with the output.

Each row records the input, output, error, duration, tokens and cost, and `claude_call_id`: the model call that asked for the tool, once the MCP server passes it (always empty for now). Validation errors name the JSON path and the expected type, such as `$.items[2].title: expected string`, and never quote the value.

### Permission levels

| Level | Meaning |
|---|---|
| `read` | Reads data; no side effects. |
| `write` | Changes Waypoint's own data; reversible and logged with undo. |
| `external` | Leaves Waypoint (sends, submits, calls out); always passes an approval gate. |

Levels are stored on every tool call but not enforced yet. The approval gate for `external` tools is a later job.

### Supported schema subset

Tool schemas are checked by a small in-repo checker (`api/src/tools/schema.rs`), not a JSON Schema crate. Supported keywords:

- `type`: one of `object`, `array`, `string`, `integer`, `number`, `boolean`, `null`, or a non-empty array of them (for example `["string", "null"]`),
- `properties`, `required`, `additionalProperties` (`false` or `true` only), `items`, `enum`,
- `description` and `title` (ignored when validating).

Any other keyword (`oneOf`, `$ref`, `minimum`, `pattern`, …) makes registration fail.

## Routes

Both routes are read-only, answer only a local `Host` (403 otherwise) and answer 500 on a database error.

- `GET /runs?limit=&before=`: runs newest first (`id` descending). `limit` defaults to 50 and is clamped to 1..=200. `before` returns only runs with `id < before`, so the next page is `?before=<last id>`. With no runs the body is `[]`.
- `GET /runs/{id}`: the run with `steps` ordered by `ordinal`, each with its `tool_calls` ordered by `id`. An unknown id is 404.

There are no routes to start, resume or cancel runs yet.

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
- a step succeeding or failing (a failed step's event already has the run's settled status: `queued` for a retry or a usage-limit deferral, `failed` otherwise),
- the run succeeding, or failing because its workflow isn't in the runtime,
- the best-effort settle after a database error.

So a one-step run whose step makes one tool call sends `queued`, `running` (run), `running` (step), `running` (tool returned), `running` (step succeeded) and `succeeded`.

`runs::publish(pool, bus, run_id)` reads the row and publishes it. It is best effort: if the read fails it logs a warning and publishes nothing, and the run carries on. Like every event there is no replay; after `app.connected` or `app.lagged` a client refetches.
