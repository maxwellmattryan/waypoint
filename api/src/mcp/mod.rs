//! Waypoint's MCP server, which hands a step's model call the registered tools and
//! nothing else (see docs/agent-runtime.md).
//!
//! The protocol is served inside the API process over a Unix socket that exists for one
//! `claude -p` call. The CLI starts the `waypoint-mcp` relay bin from the generated config,
//! and the relay copies its stdio to and from the socket. Every `tools/call` goes through
//! [`Registry::invoke_metered`], so it is recorded in `tool_calls` on the calling step.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value, json};
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};

use crate::claude::{ClaudeError, ClaudeRunner, MCP_SERVER_NAME};
use crate::tools::{Registry, Spend, ToolCallError};

/// The longest request line read; a longer one closes the connection.
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
/// The protocol version answered when the client asks for one this server doesn't know.
const LATEST_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What one session's tool calls created and spent.
#[derive(Debug, Default)]
struct SessionState {
    tool_call_ids: Vec<i64>,
    spend: Spend,
}

/// Serves MCP for one run step: every tool call is recorded against `step_id`. Clones
/// share the session's record of tool call ids and spend.
#[derive(Clone)]
pub struct Server {
    pool: PgPool,
    runner: ClaudeRunner,
    tools: Arc<Registry>,
    step_id: i64,
    state: Arc<Mutex<SessionState>>,
}

/// A JSON-RPC error code and message.
type RpcError = (i64, &'static str);

impl Server {
    pub fn new(pool: PgPool, runner: ClaudeRunner, tools: Arc<Registry>, step_id: i64) -> Self {
        Self {
            pool,
            runner,
            tools,
            step_id,
            state: Arc::default(),
        }
    }

    /// Serves one connection: newline-delimited JSON-RPC 2.0, one request at a time in
    /// arrival order. Returns once the client closes its side and the last response is
    /// written, or when a line is over [`MAX_LINE_BYTES`].
    pub async fn serve<S: AsyncRead + AsyncWrite + Unpin>(&self, stream: S) {
        let (read, mut write) = tokio::io::split(stream);
        let mut reader = BufReader::new(read);
        let mut line = Vec::new();
        loop {
            line.clear();
            let limit = u64::try_from(MAX_LINE_BYTES).unwrap_or(u64::MAX) + 1;
            match (&mut reader).take(limit).read_until(b'\n', &mut line).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(err) => {
                    rocket::warn!("mcp: cannot read a request: {err}");
                    break;
                }
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.len() > MAX_LINE_BYTES {
                rocket::warn!("mcp: a request line is over {MAX_LINE_BYTES} bytes; closing");
                break;
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let Some(response) = self.handle(&line).await else {
                continue;
            };
            let mut out = response.to_string();
            out.push('\n');
            if write.write_all(out.as_bytes()).await.is_err() || write.flush().await.is_err() {
                break;
            }
        }
        let _ = write.shutdown().await;
    }

    /// The response to one line, or `None` for a notification.
    async fn handle(&self, line: &[u8]) -> Option<Value> {
        let Ok(request) = serde_json::from_slice::<Value>(line) else {
            return Some(error(Value::Null, (PARSE_ERROR, "parse error")));
        };
        let Value::Object(request) = request else {
            return Some(error(Value::Null, (INVALID_REQUEST, "invalid request")));
        };
        // A message without an id is a notification and gets no response.
        let id = request.get("id")?.clone();
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return Some(error(id, (INVALID_REQUEST, "invalid request")));
        };
        let params = request.get("params");
        let result = match method {
            "initialize" => Ok(initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.list()),
            "tools/call" => self.call(params).await,
            _ => Err((METHOD_NOT_FOUND, "method not found")),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(err) => error(id, err),
        })
    }

    fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .tools
            .describe()
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect();
        json!({"tools": tools})
    }

    async fn call(&self, params: Option<&Value>) -> Result<Value, RpcError> {
        let Some(name) = params
            .and_then(|params| params.get("name"))
            .and_then(Value::as_str)
        else {
            return Err((INVALID_PARAMS, "invalid params"));
        };
        let arguments = params
            .and_then(|params| params.get("arguments"))
            .filter(|arguments| !arguments.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let (spend, id, result) = self
            .tools
            .invoke_metered(
                &self.pool,
                &self.runner,
                self.step_id,
                None,
                name,
                arguments,
            )
            .await;
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.spend.add(spend);
            state.tool_call_ids.extend(id);
        }
        match result {
            Ok(output) => Ok(text_result(output.to_string(), false)),
            Err(ToolCallError::UnknownTool) => Err((INVALID_PARAMS, "no tool has this name")),
            // The registry's messages never quote input values.
            Err(err) => Ok(text_result(err.to_string(), true)),
        }
    }

    fn take_state(&self) -> SessionState {
        std::mem::take(&mut *self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

fn initialize(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = requested
        .filter(|version| SUPPORTED_PROTOCOLS.contains(version))
        .unwrap_or(LATEST_PROTOCOL);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": MCP_SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
    })
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn error(id: Value, (code, message): RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The MCP server for one `claude -p` call: a 0700 temp dir holding the socket the API
/// listens on and the config that points the CLI at the relay. Stopped with
/// [`Session::finish`].
pub(crate) struct Session {
    dir: Option<TempDir>,
    config: PathBuf,
    server: Server,
    stop: Option<oneshot::Sender<()>>,
    accept: Option<JoinHandle<()>>,
}

impl Session {
    /// Creates the dir, binds `mcp.sock`, writes `mcp.json` and starts accepting
    /// connections. Fails before anything is spawned when `mcp_bin` isn't a file.
    pub(crate) fn start(server: Server, mcp_bin: &Path) -> Result<Self, ClaudeError> {
        if !mcp_bin.is_file() {
            return Err(setup_error(format!(
                "the MCP relay {} does not exist; build it with `cargo build -p waypoint-api --bins`",
                mcp_bin.display()
            )));
        }
        let mcp_bin = std::path::absolute(mcp_bin).map_err(|err| {
            setup_error(format!(
                "the MCP relay {} is not a usable path: {err}",
                mcp_bin.display()
            ))
        })?;
        let dir = tempfile::Builder::new()
            .prefix("waypoint-mcp-")
            .tempdir()
            .map_err(|err| setup_error(format!("cannot create the MCP temp dir: {err}")))?;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))
            .map_err(|err| setup_error(format!("cannot restrict the MCP temp dir: {err}")))?;
        let socket = dir.path().join("mcp.sock");
        let listener = UnixListener::bind(&socket).map_err(|err| {
            setup_error(format!(
                "cannot bind the MCP socket {}: {err}",
                socket.display()
            ))
        })?;
        let config = dir.path().join("mcp.json");
        fs::write(&config, mcp_config(&mcp_bin, &socket)?)
            .map_err(|err| setup_error(format!("cannot write the MCP config: {err}")))?;

        let (stop, mut stopped) = oneshot::channel::<()>();
        let serving = server.clone();
        let accept = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => {
                            let server = serving.clone();
                            connections.spawn(async move { server.serve(stream).await });
                        }
                        Err(err) => {
                            rocket::warn!("mcp: cannot accept a connection: {err}");
                            break;
                        }
                    },
                }
            }
            drop(listener);
            // Calls already in flight finish and write their rows; each connection ends
            // when the relay goes, which the runner's process-group kill makes sure of.
            while connections.join_next().await.is_some() {}
        });
        Ok(Self {
            dir: Some(dir),
            config,
            server,
            stop: Some(stop),
            accept: Some(accept),
        })
    }

    /// The generated `mcp.json`.
    pub(crate) fn config(&self) -> &Path {
        &self.config
    }

    /// Stops accepting, waits for open connections to finish, removes the dir and returns
    /// the `tool_calls` ids this session created and what their model calls spent.
    pub(crate) async fn finish(mut self) -> (Vec<i64>, Spend) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(accept) = self.accept.take()
            && let Err(err) = accept.await
        {
            rocket::warn!("mcp: the session's accept loop failed: {err}");
        }
        if let Some(dir) = self.dir.take()
            && let Err(err) = dir.close()
        {
            rocket::warn!("mcp: cannot remove the session's temp dir: {err}");
        }
        let state = self.server.take_state();
        (state.tool_call_ids, state.spend)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Only when the caller was cancelled before `finish`; the connections go with it.
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
    }
}

/// `{"mcpServers":{"waypoint":{"type":"stdio","command":…,"args":["--socket",…]}}}`.
fn mcp_config(mcp_bin: &Path, socket: &Path) -> Result<String, ClaudeError> {
    let utf8 = |path: &Path| {
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| setup_error(format!("{} is not a UTF-8 path", path.display())))
    };
    let mut servers = Map::new();
    servers.insert(
        MCP_SERVER_NAME.to_owned(),
        json!({"type": "stdio", "command": utf8(mcp_bin)?, "args": ["--socket", utf8(socket)?]}),
    );
    Ok(json!({"mcpServers": servers}).to_string())
}

fn setup_error(message: String) -> ClaudeError {
    ClaudeError::Process { message }
}
