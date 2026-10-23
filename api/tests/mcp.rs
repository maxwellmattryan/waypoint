//! Waypoint's MCP server: the protocol over an in-memory stream, and agent calls that
//! reach it through `fake-claude.sh` (mode `mcp`) and the real `waypoint-mcp` relay.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use waypoint_api::claude::{ClaudeRequest, ClaudeRunner};
use waypoint_api::events::{Event, EventBus, Subscriber};
use waypoint_api::jobs::{Wake, Worker};
use waypoint_api::mcp::{MAX_LINE_BYTES, Server};
use waypoint_api::runs::{self, Run, RunStatus, Runtime, StepContext, StepError, Workflow};
use waypoint_api::tools::{Permission, Registry, Tool, ToolContext, ToolError};

const TIMEOUT: Duration = Duration::from_secs(20);
const RELAY: &str = env!("CARGO_BIN_EXE_waypoint-mcp");

fn request(purpose: &str, prompt: &str) -> ClaudeRequest {
    ClaudeRequest {
        prompt: prompt.into(),
        purpose: purpose.into(),
        model: None,
    }
}

fn recorded(record: &Path, name: &str) -> String {
    fs::read_to_string(record.join(name)).unwrap_or_else(|err| panic!("record {name}: {err}"))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupInput {
    q: String,
}

/// A `read` tool that upper-cases `q`.
struct Lookup;

#[rocket::async_trait]
impl Tool for Lookup {
    const NAME: &'static str = "lookup";
    const DESCRIPTION: &'static str = "Upper-cases a query";
    const PERMISSION: Permission = Permission::Read;
    type Input = LookupInput;
    type Output = Value;

    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"q": {"type": "string"}},
            "required": ["q"],
            "additionalProperties": false
        })
    }

    fn output_schema() -> Value {
        json!({"type": "object", "properties": {"answer": {"type": "string"}}})
    }

    async fn call(
        &self,
        _ctx: &mut ToolContext<'_>,
        input: LookupInput,
    ) -> Result<Value, ToolError> {
        Ok(json!({"answer": input.q.to_uppercase()}))
    }
}

/// A `read` tool that makes one model call.
struct Summarize;

#[rocket::async_trait]
impl Tool for Summarize {
    const NAME: &'static str = "summarize";
    const DESCRIPTION: &'static str = "Asks the model once";
    const PERMISSION: Permission = Permission::Read;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object", "additionalProperties": false})
    }

    fn output_schema() -> Value {
        json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]})
    }

    async fn call(&self, ctx: &mut ToolContext<'_>, _input: Value) -> Result<Value, ToolError> {
        let response = ctx.call(&request("test.summarize", "ping")).await?;
        Ok(json!({"text": response.text}))
    }
}

/// A `write` tool whose handler always fails.
struct Broken;

#[rocket::async_trait]
impl Tool for Broken {
    const NAME: &'static str = "broken";
    const DESCRIPTION: &'static str = "Always fails";
    const PERMISSION: Permission = Permission::Write;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object"})
    }

    fn output_schema() -> Value {
        json!({"type": "object"})
    }

    async fn call(&self, _ctx: &mut ToolContext<'_>, _input: Value) -> Result<Value, ToolError> {
        Err(ToolError::Failed("the outbox is closed".into()))
    }
}

/// An `external` tool that counts how often its handler ran and sends nothing. Agents see
/// it, and a call waits for the user's approval without running the handler.
struct Sender(Arc<AtomicUsize>);

#[rocket::async_trait]
impl Tool for Sender {
    const NAME: &'static str = "send";
    const DESCRIPTION: &'static str = "Sends a message";
    const PERMISSION: Permission = Permission::External;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object"})
    }

    fn output_schema() -> Value {
        json!({"type": "object"})
    }

    async fn call(&self, _ctx: &mut ToolContext<'_>, _input: Value) -> Result<Value, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"sent": false}))
    }
}

/// A `read` tool whose output breaks its own schema.
struct Liar;

#[rocket::async_trait]
impl Tool for Liar {
    const NAME: &'static str = "liar";
    const DESCRIPTION: &'static str = "Returns the wrong shape";
    const PERMISSION: Permission = Permission::Read;
    type Input = Value;
    type Output = Value;

    fn input_schema() -> Value {
        json!({"type": "object"})
    }

    fn output_schema() -> Value {
        json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]})
    }

    async fn call(&self, _ctx: &mut ToolContext<'_>, _input: Value) -> Result<Value, ToolError> {
        Ok(json!({"n": "seven"}))
    }
}

/// One step that makes an agent call with `prompt` and outputs the reply's text, or the
/// error's kind and message, so the step succeeds either way.
struct Agent(&'static str);

#[rocket::async_trait]
impl Workflow for Agent {
    fn name(&self) -> &'static str {
        "agent"
    }

    fn steps(&self) -> &'static [&'static str] {
        &["ask"]
    }

    async fn run_step(
        &self,
        ctx: &mut StepContext<'_>,
        _step: &str,
        _input: Value,
    ) -> Result<Value, StepError> {
        Ok(
            match ctx.call_with_tools(&request("test.agent", self.0)).await {
                Ok(response) => json!({"text": response.text}),
                Err(err) => json!({"kind": err.kind(), "error": err.to_string()}),
            },
        )
    }
}

fn registry(tools: &[&str]) -> Registry {
    registry_counting(tools, &Arc::default())
}

/// [`registry`], with `send`'s handler runs counted in `sent`.
fn registry_counting(tools: &[&str], sent: &Arc<AtomicUsize>) -> Registry {
    let mut registry = Registry::default();
    for name in tools {
        match *name {
            "lookup" => registry.register(Lookup),
            "summarize" => registry.register(Summarize),
            "broken" => registry.register(Broken),
            "liar" => registry.register(Liar),
            "send" => registry.register(Sender(sent.clone())),
            other => panic!("no test tool {other}"),
        }
        .expect("register");
    }
    registry
}

/// A worker using `runner` and `runtime`, plus what a test needs to start runs on it.
struct Harness {
    pool: PgPool,
    bus: EventBus,
    wake: Wake,
    runtime: Runtime,
    _stop: watch::Sender<bool>,
}

impl Harness {
    fn start(pool: &PgPool, runner: ClaudeRunner, runtime: Runtime) -> (Self, Subscriber) {
        let bus = EventBus::new();
        let subscriber = bus.subscribe();
        let wake = Wake::default();
        let (stop, stopped) = watch::channel(false);
        Worker {
            pool: pool.clone(),
            runner,
            bus: bus.clone(),
            config: common::fast_config(),
            wake: wake.0.clone(),
            runtime: runtime.clone(),
            google: waypoint_api::google::Google::new(Default::default()),
            capture: waypoint_api::capture::Capturer::new(Default::default()),
        }
        .spawn(stopped);
        let harness = Self {
            pool: pool.clone(),
            bus,
            wake,
            runtime,
            _stop: stop,
        };
        (harness, subscriber)
    }

    /// Starts an `agent` run and waits for its job to finish.
    async fn run(&self, subscriber: &mut Subscriber) -> Run {
        self.run_with_events(subscriber).await.0
    }

    /// [`Harness::run`], also returning the events seen until the job finished.
    async fn run_with_events(&self, subscriber: &mut Subscriber) -> (Run, Vec<Event>) {
        let started = runs::start(
            &self.pool,
            &self.bus,
            &self.wake,
            &self.runtime,
            "agent",
            json!({}),
        )
        .await
        .expect("start the run");
        let job_id = started.job_id.expect("job_id");
        let events = common::events(subscriber, TIMEOUT, |event| {
            matches!(event, Event::JobSucceeded { job } | Event::JobFailed { job } if job.id == job_id)
        })
        .await;
        let run = runs::list_runs(&self.pool, 200, None)
            .await
            .expect("list runs")
            .into_iter()
            .find(|run| run.id == started.id)
            .expect("the run exists");
        (run, events)
    }
}

fn agent_runtime(prompt: &'static str, tools: &[&str]) -> Runtime {
    Runtime::new(vec![Arc::new(Agent(prompt))], registry(tools)).expect("valid runtime")
}

/// `(tool, status, output, claude_call_id)` of a run's tool calls, by id.
type ToolCallRow = (String, String, Option<Value>, Option<i64>);

async fn tool_calls(pool: &PgPool, run_id: i64) -> Vec<ToolCallRow> {
    sqlx::query_as(
        "SELECT t.tool, t.status, t.output, t.claude_call_id \
         FROM tool_calls AS t JOIN run_steps AS s ON s.id = t.run_step_id \
         WHERE s.run_id = $1 ORDER BY t.id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read tool calls")
}

/// What a successful call spent, as the step counts it: `(id, input, output, cost)`.
async fn call_spend(pool: &PgPool, purpose: &str) -> (i64, i64, i64, f64) {
    sqlx::query_as(
        "SELECT id, COALESCE(input_tokens, 0) + COALESCE(cache_creation_input_tokens, 0) \
         + COALESCE(cache_read_input_tokens, 0), COALESCE(output_tokens, 0), \
         COALESCE(cost_usd, 0) FROM claude_calls WHERE purpose = $1",
    )
    .bind(purpose)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("the {purpose} call: {err}"))
}

fn text_result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn ok(id: i64, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn initialized(version: &str) -> Value {
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "waypoint", "version": env!("CARGO_PKG_VERSION")},
    })
}

/// Sends `input` to `server` over an in-memory stream, closes the client's write side
/// and returns everything the server wrote before closing.
async fn exchange(server: Server, input: Vec<u8>) -> Vec<u8> {
    let (client, stream) = tokio::io::duplex(64 * 1024);
    let serving = tokio::spawn(async move { server.serve(stream).await });
    let (mut read, mut write) = tokio::io::split(client);
    let writing = tokio::spawn(async move {
        // The server may close first (a line over the limit), so a failed write is fine.
        let _ = write.write_all(&input).await;
        let _ = write.shutdown().await;
    });
    let mut out = Vec::new();
    tokio::time::timeout(TIMEOUT, read.read_to_end(&mut out))
        .await
        .expect("the server closes the stream")
        .expect("read the responses");
    writing.await.expect("writer");
    serving.await.expect("server");
    out
}

#[sqlx::test]
async fn protocol_round_trips(pool: PgPool) {
    let runtime = agent_runtime("unused", &["lookup", "broken", "liar"]);
    let run = runs::start(
        &pool,
        &EventBus::new(),
        &Wake::default(),
        &runtime,
        "agent",
        json!({}),
    )
    .await
    .expect("start the run");
    let step_id: i64 = sqlx::query_scalar("SELECT id FROM run_steps WHERE run_id = $1")
        .bind(run.id)
        .fetch_one(&pool)
        .await
        .expect("the step");
    let fake = common::fake("success", TIMEOUT);
    let sent = Arc::new(AtomicUsize::new(0));
    let tools = Arc::new(registry_counting(
        &["lookup", "broken", "liar", "send"],
        &sent,
    ));
    let server = Server::new(
        pool.clone(),
        fake.runner.clone(),
        tools,
        EventBus::new(),
        run.id,
        step_id,
    );

    let call = |id: i64, params: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params}).to_string()
    };
    let lookup_schema = Lookup::input_schema();
    let object = json!({"type": "object"});
    // Request line → the response it gets, or `None` for no response.
    let cases: Vec<(String, Option<Value>)> = vec![
        (
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}).to_string(),
            Some(ok(1, initialized("2025-03-26"))),
        ),
        (
            json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}).to_string(),
            Some(ok(2, initialized("2025-06-18"))),
        ),
        (
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            None,
        ),
        (
            json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}).to_string(),
            Some(ok(3, json!({}))),
        ),
        (
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list"}).to_string(),
            Some(ok(
                4,
                json!({"tools": [
                    {"name": "broken", "description": "Always fails", "inputSchema": object},
                    {"name": "liar", "description": "Returns the wrong shape", "inputSchema": object},
                    {"name": "lookup", "description": "Upper-cases a query", "inputSchema": lookup_schema},
                    {"name": "send", "description": "Sends a message", "inputSchema": object},
                ]}),
            )),
        ),
        (
            call(5, json!({"name": "lookup", "arguments": {"q": "hi"}})),
            Some(ok(5, text_result(r#"{"answer":"HI"}"#, false))),
        ),
        (
            call(6, json!({"name": "lookup", "arguments": {"q": 5}})),
            Some(ok(
                6,
                text_result("invalid tool input: $.q: expected string", true),
            )),
        ),
        (
            call(7, json!({"name": "broken"})),
            Some(ok(
                7,
                text_result("the tool failed: the outbox is closed", true),
            )),
        ),
        (
            call(8, json!({"name": "liar", "arguments": {}})),
            Some(ok(
                8,
                text_result("invalid tool output: $.n: expected integer", true),
            )),
        ),
        (
            call(9, json!({"name": "nope", "arguments": {}})),
            Some(err(json!(9), -32602, "no tool has this name")),
        ),
        (
            call(10, json!({"arguments": {}})),
            Some(err(json!(10), -32602, "invalid params")),
        ),
        (
            call(11, json!({"name": 5})),
            Some(err(json!(11), -32602, "invalid params")),
        ),
        // An `external` tool waits for the user's approval without running its handler.
        (
            call(15, json!({"name": "send", "arguments": {}})),
            Some(ok(
                15,
                text_result(
                    "this action is waiting for the user's approval; end your turn without retrying it",
                    true,
                ),
            )),
        ),
        (
            json!({"jsonrpc": "2.0", "id": "twelve", "method": "resources/list"}).to_string(),
            Some(err(json!("twelve"), -32601, "method not found")),
        ),
        (
            "not json".to_string(),
            Some(err(Value::Null, -32700, "parse error")),
        ),
        (
            json!([{"jsonrpc": "2.0", "id": 13, "method": "ping"}]).to_string(),
            Some(err(Value::Null, -32600, "invalid request")),
        ),
        ("5".to_string(), Some(err(Value::Null, -32600, "invalid request"))),
        // The last request is still answered after the client closes its side.
        (
            call(14, json!({"name": "lookup", "arguments": {"q": "bye"}})),
            Some(ok(14, text_result(r#"{"answer":"BYE"}"#, false))),
        ),
    ];

    let mut input = String::new();
    for (line, _) in &cases {
        input.push_str(line);
        input.push('\n');
    }
    let out = exchange(server.clone(), input.into_bytes()).await;
    let responses: Vec<Value> = String::from_utf8(out)
        .expect("utf-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is JSON"))
        .collect();
    let expected: Vec<Value> = cases.into_iter().filter_map(|(_, want)| want).collect();
    assert_eq!(responses.len(), expected.len(), "{responses:#?}");
    for (got, want) in responses.iter().zip(&expected) {
        assert_eq!(got, want);
    }

    // A line over the limit closes the connection without a response.
    let long = vec![b'a'; MAX_LINE_BYTES + 1];
    assert!(exchange(server, long).await.is_empty());

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT tool, status FROM tool_calls WHERE run_step_id = $1 ORDER BY id")
            .bind(step_id)
            .fetch_all(&pool)
            .await
            .expect("read tool calls");
    let rows: Vec<(&str, &str)> = rows
        .iter()
        .map(|(tool, status)| (tool.as_str(), status.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("lookup", "succeeded"),
            ("lookup", "invalid_input"),
            ("broken", "failed"),
            ("liar", "invalid_output"),
            ("nope", "unknown_tool"),
            ("send", "awaiting_approval"),
            ("lookup", "succeeded"),
        ],
        "a call without a name writes no row"
    );
    assert_eq!(sent.load(Ordering::SeqCst), 0, "send's handler never ran");
    let pending: Vec<(String, String, Value)> =
        sqlx::query_as("SELECT tool, status, input FROM pending_actions WHERE run_step_id = $1")
            .bind(step_id)
            .fetch_all(&pool)
            .await
            .expect("read pending actions");
    assert_eq!(pending, [("send".into(), "pending".into(), json!({}))]);
}

#[sqlx::test]
async fn agent_tool_calls_are_recorded_on_the_step(pool: PgPool) {
    let fake = common::fake("mcp", TIMEOUT);
    let runtime = agent_runtime("use the tools", &["lookup", "summarize"]);
    let (harness, mut subscriber) = Harness::start(&pool, fake.runner.clone(), runtime);
    let (run, events) = harness.run_with_events(&mut subscriber).await;
    assert_eq!(run.output, Some(json!({"text": "pong"})), "{run:?}");

    // One `run.updated` per registry call, each before the step finishes.
    let statuses: Vec<RunStatus> = events
        .iter()
        .filter_map(|event| match event {
            Event::RunUpdated { run: updated } if updated.id == run.id => Some(updated.status),
            _ => None,
        })
        .collect();
    use RunStatus::{Queued, Running, Succeeded};
    assert_eq!(
        statuses,
        [
            Queued, Running, Running, Running, Running, Running, Running, Running, Succeeded
        ],
        "start, run running, step running, four tool calls, step succeeded, run succeeded"
    );

    // The CLI got the isolated argv with the MCP flags last.
    let argv: Vec<String> = recorded(&fake.record, "argv")
        .lines()
        .map(str::to_owned)
        .collect();
    let base = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
    ];
    assert_eq!(argv[..base.len()], base);
    assert_eq!(argv.len(), base.len() + 4, "{argv:?}");
    let tail = &argv[base.len()..];
    assert_eq!(tail[0], "--mcp-config");
    assert_eq!(tail[2..], ["--allowedTools", "mcp__waypoint"]);
    assert!(!argv.iter().any(|arg| arg == "--bare"));

    // The config names one stdio server: the relay, pointed at the session's socket.
    let config_path = PathBuf::from(&tail[1]);
    let mcp_dir = config_path.parent().expect("the MCP dir").to_path_buf();
    let config: Value =
        serde_json::from_str(&recorded(&fake.record, "mcp_config")).expect("config is JSON");
    assert_eq!(
        config,
        json!({"mcpServers": {"waypoint": {
            "type": "stdio",
            "command": RELAY,
            "args": ["--socket", mcp_dir.join("mcp.sock")],
        }}})
    );
    assert_eq!(recorded(&fake.record, "mcp_dir_mode").trim(), "700");
    let cwd = PathBuf::from(recorded(&fake.record, "cwd").trim());
    assert_eq!(
        recorded(&fake.record, "ls"),
        "",
        "the CLI's cwd starts empty"
    );
    assert_ne!(cwd, mcp_dir);
    assert!(!cwd.exists(), "{cwd:?} is removed");
    assert!(!mcp_dir.exists(), "{mcp_dir:?} is removed");

    // What the relay passed back to the CLI.
    let responses: Vec<Value> = recorded(&fake.record, "mcp")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is JSON"))
        .collect();
    let lookup_schema = Lookup::input_schema();
    let summarize_schema = Summarize::input_schema();
    assert_eq!(
        responses,
        [
            ok(1, initialized("2025-06-18")),
            ok(
                2,
                json!({"tools": [
                    {"name": "lookup", "description": "Upper-cases a query", "inputSchema": lookup_schema},
                    {"name": "summarize", "description": "Asks the model once", "inputSchema": summarize_schema},
                ]})
            ),
            ok(3, text_result(r#"{"answer":"HI"}"#, false)),
            ok(
                4,
                text_result("invalid tool input: $.q: expected string", true)
            ),
            err(json!(5), -32602, "no tool has this name"),
            ok(6, text_result(r#"{"text":"pong"}"#, false)),
        ]
    );

    // Each tool call is on the step and linked to the call that asked for it.
    let (outer, outer_in, outer_out, outer_cost) = call_spend(&pool, "test.agent").await;
    let (_, tool_in, tool_out, tool_cost) = call_spend(&pool, "test.summarize").await;
    let rows = tool_calls(&pool, run.id).await;
    assert_eq!(
        rows,
        [
            (
                "lookup".into(),
                "succeeded".into(),
                Some(json!({"answer": "HI"})),
                Some(outer)
            ),
            ("lookup".into(), "invalid_input".into(), None, Some(outer)),
            ("nope".into(), "unknown_tool".into(), None, Some(outer)),
            (
                "summarize".into(),
                "succeeded".into(),
                Some(json!({"text": "pong"})),
                Some(outer)
            ),
        ]
    );

    // The step and the run count the outer call and the tool's own call.
    let step: (i64, i64, f64, Option<i64>) = sqlx::query_as(
        "SELECT input_tokens, output_tokens, cost_usd, claude_call_id FROM run_steps \
         WHERE run_id = $1",
    )
    .bind(run.id)
    .fetch_one(&pool)
    .await
    .expect("read the step");
    assert!(tool_in > 0 && tool_out > 0 && tool_cost > 0.0);
    assert_eq!(
        (step.0, step.1, step.3),
        (outer_in + tool_in, outer_out + tool_out, Some(outer))
    );
    assert!((step.2 - (outer_cost + tool_cost)).abs() < 1e-12);
    assert_eq!((run.input_tokens, run.output_tokens), (step.0, step.1));
    assert!((run.cost_usd - step.2).abs() < 1e-12);
}

#[sqlx::test]
async fn agent_external_call_parks_the_run(pool: PgPool) {
    let fake = common::fake("mcp_external", TIMEOUT);
    let sent = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::new(
        vec![Arc::new(Agent("send a message"))],
        registry_counting(&["send"], &sent),
    )
    .expect("valid runtime");
    let (harness, mut subscriber) = Harness::start(&pool, fake.runner.clone(), runtime);
    let run = harness.run(&mut subscriber).await;

    // The step's code returned `Ok`, but the call waits for approval, so the run parks.
    assert_eq!(run.status, RunStatus::AwaitingApproval, "{run:?}");
    assert_eq!((run.output, run.error), (None, None));
    let step: (String, Option<Value>) =
        sqlx::query_as("SELECT status, output FROM run_steps WHERE run_id = $1")
            .bind(run.id)
            .fetch_one(&pool)
            .await
            .expect("read the step");
    assert_eq!(step, ("awaiting_approval".into(), None));

    let responses: Vec<Value> = recorded(&fake.record, "mcp")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response is JSON"))
        .collect();
    assert_eq!(
        responses[2],
        ok(
            3,
            text_result(
                "this action is waiting for the user's approval; end your turn without retrying it",
                true
            )
        )
    );
    let rows = tool_calls(&pool, run.id).await;
    let (outer, ..) = call_spend(&pool, "test.agent").await;
    assert_eq!(
        rows,
        [("send".into(), "awaiting_approval".into(), None, Some(outer))]
    );
    let pending: Vec<(String, String, Value)> =
        sqlx::query_as("SELECT tool, status, input FROM pending_actions WHERE run_id = $1")
            .bind(run.id)
            .fetch_all(&pool)
            .await
            .expect("read pending actions");
    assert_eq!(
        pending,
        [("send".into(), "pending".into(), json!({"to": "a"}))]
    );
    assert_eq!(sent.load(Ordering::SeqCst), 0, "send's handler never ran");
}

#[sqlx::test]
async fn missing_relay_bin_fails_before_spawning(pool: PgPool) {
    let fake = common::fake("mcp", TIMEOUT);
    let missing = fake.record.join("nowhere/waypoint-mcp");
    let runner = fake.runner.clone().with_mcp_bin(&missing);
    let (harness, mut subscriber) =
        Harness::start(&pool, runner, agent_runtime("unused", &["lookup"]));
    let run = harness.run(&mut subscriber).await;

    let output = run.output.expect("the step reports the error");
    assert_eq!(output["kind"], "process_failed", "{output}");
    let error = output["error"].as_str().expect("error text");
    assert!(error.contains(&missing.display().to_string()), "{error}");
    assert!(
        error.contains("cargo build -p waypoint-api --bins"),
        "{error}"
    );
    assert!(
        !fake.record.join("argv").exists(),
        "claude was never started"
    );
    let calls: i64 = sqlx::query_scalar("SELECT count(*) FROM claude_calls")
        .fetch_one(&pool)
        .await
        .expect("count calls");
    assert_eq!(calls, 0);
}

#[test]
fn relay_reports_unreachable_socket() {
    let dir = tempfile::tempdir().expect("temp dir");
    let socket = dir.path().join("nope.sock");
    let out = std::process::Command::new(RELAY)
        .arg("--socket")
        .arg(&socket)
        .output()
        .expect("run the relay");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot connect"), "{stderr}");
    assert!(stderr.contains(&socket.display().to_string()), "{stderr}");
    assert_eq!(stderr.trim().lines().count(), 1, "{stderr}");
    assert!(out.stdout.is_empty());

    let out = std::process::Command::new(RELAY)
        .arg("--port")
        .output()
        .expect("run the relay");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: waypoint-mcp --socket"));
}

/// Manual check against the real, logged-in `claude` CLI (spends subscription usage):
/// `cargo build -p waypoint-api --bins && cargo test -p waypoint-api --test mcp -- --ignored real_claude_calls_a_waypoint_tool`
#[sqlx::test]
#[ignore = "calls the real claude CLI with your subscription"]
async fn real_claude_calls_a_waypoint_tool(pool: PgPool) {
    let mut runner = ClaudeRunner::from_env().expect("runner settings");
    if std::env::var_os("WAYPOINT_MCP_BIN").is_none() {
        // The test binary lives in target/*/deps, not beside the relay.
        runner = runner.with_mcp_bin(RELAY);
    }
    let runtime = agent_runtime(
        "Call the lookup tool with q=hi and reply with its answer only",
        &["lookup"],
    );
    let (harness, mut subscriber) = Harness::start(&pool, runner, runtime);
    let run = harness.run(&mut subscriber).await;
    let output = run.output.expect("the step's output");
    let text = output["text"].as_str().unwrap_or_default();
    assert!(text.contains("HI"), "{output}");
    let rows = tool_calls(&pool, run.id).await;
    assert!(
        rows.iter()
            .any(|(tool, status, ..)| tool == "lookup" && status == "succeeded"),
        "{rows:?}"
    );
}
