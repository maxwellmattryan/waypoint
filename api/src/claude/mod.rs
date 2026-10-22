//! The only way Waypoint calls a model: it spawns the user's logged-in `claude` CLI,
//! isolated from the filesystem and any repo context, so calls bill the Claude
//! subscription and never the API. Every call writes one row to `claude_calls`.

mod args;
mod log;
mod stream;

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use self::stream::Completed;

/// Binary used when `WAYPOINT_CLAUDE_BIN` is unset; resolved through `PATH`.
pub const DEFAULT_BIN: &str = "claude";
/// Timeout used when `WAYPOINT_CLAUDE_TIMEOUT_SECS` is unset.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// Environment variables removed from the child: keys and switches that would move
/// billing or traffic away from the subscription, and Waypoint's database URL.
const SCRUBBED_ENV: [&str; 7] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "DATABASE_URL",
];
/// Prefixes of Waypoint's own settings and secrets, also withheld from the child.
const SCRUBBED_ENV_PREFIXES: [&str; 2] = ["POSTGRES_", "WAYPOINT_"];
/// How much stderr (its tail) and how much of an error message are kept.
pub(crate) const MAX_ERROR_BYTES: usize = 2048;
/// Most stdout read from one call; more is a protocol error and the child is killed.
const MAX_STDOUT_BYTES: u64 = 16 * 1024 * 1024;

/// Runs isolated `claude -p` calls.
#[derive(Debug, Clone)]
pub struct ClaudeRunner {
    bin: PathBuf,
    timeout: Duration,
}

/// One single-turn model call.
#[derive(Debug, Clone)]
pub struct ClaudeRequest {
    pub prompt: String,
    /// Short label for the call log, such as `dev.ping`.
    pub purpose: String,
    pub model: Option<String>,
}

/// Token usage as reported by the CLI.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
}

/// A successful call.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeResponse {
    pub text: String,
    pub structured: Option<Value>,
    pub model: Option<String>,
    pub usage: Usage,
    /// Wall-clock time of the whole call, measured by the runner.
    pub duration_ms: u64,
    /// The CLI's reported `total_cost_usd`.
    pub cost_usd: Option<f64>,
    /// Id of the `claude_calls` row, or `None` if logging failed.
    pub log_id: Option<i64>,
}

/// Why a call failed.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeError {
    /// The subscription's usage limit was hit; callers should back off.
    UsageLimit { message: String },
    /// The CLI reported an error result.
    Cli { subtype: String, message: String },
    /// The structured output was missing or did not fit the caller's type.
    SchemaMismatch { message: String },
    /// The call ran longer than the runner's timeout and was killed.
    Timeout { after: Duration },
    /// The CLI could not be started or exited without a result.
    Process { message: String },
    /// The CLI's output could not be understood.
    Protocol { message: String },
}

impl ClaudeError {
    /// Stable name of the failure, used as the call log's `status`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UsageLimit { .. } => "usage_limit",
            Self::Cli { .. } => "cli_error",
            Self::SchemaMismatch { .. } => "schema_mismatch",
            Self::Timeout { .. } => "timeout",
            Self::Process { .. } => "process_failed",
            Self::Protocol { .. } => "protocol_error",
        }
    }
}

impl fmt::Display for ClaudeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UsageLimit { message } => write!(f, "Claude usage limit hit: {message}"),
            Self::Cli { subtype, message } => {
                write!(f, "claude reported an error ({subtype}): {message}")
            }
            Self::SchemaMismatch { message } => {
                write!(f, "structured output does not match the schema: {message}")
            }
            Self::Timeout { after } => {
                write!(f, "claude did not finish within {}s", after.as_secs_f64())
            }
            Self::Process { message } => write!(f, "{message}"),
            Self::Protocol { message } => write!(f, "unexpected claude output: {message}"),
        }
    }
}

impl std::error::Error for ClaudeError {}

/// Parses a `WAYPOINT_CLAUDE_TIMEOUT_SECS` value, falling back to [`DEFAULT_TIMEOUT`]
/// when it is absent.
pub fn parse_timeout(value: Option<&str>) -> Result<Duration, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(DEFAULT_TIMEOUT),
        Some(raw) => match raw.parse::<u64>() {
            Ok(secs) if secs > 0 => Ok(Duration::from_secs(secs)),
            _ => Err(format!(
                "WAYPOINT_CLAUDE_TIMEOUT_SECS must be a positive number of seconds, got {raw:?}"
            )),
        },
    }
}

/// Model, usage and cost read from the stream, kept for the call log even on failure.
#[derive(Debug, Default)]
struct Meta {
    model: Option<String>,
    usage: Usage,
    cost_usd: Option<f64>,
}

type Outcome = (Meta, Result<Completed, ClaudeError>);

impl ClaudeRunner {
    pub fn new(bin: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            bin: bin.into(),
            timeout,
        }
    }

    /// Reads `WAYPOINT_CLAUDE_BIN` (default `claude`) and `WAYPOINT_CLAUDE_TIMEOUT_SECS`
    /// (default 300).
    pub fn from_env() -> Result<Self, String> {
        let timeout = parse_timeout(
            std::env::var("WAYPOINT_CLAUDE_TIMEOUT_SECS")
                .ok()
                .as_deref(),
        )?;
        let bin = match std::env::var_os("WAYPOINT_CLAUDE_BIN").filter(|bin| !bin.is_empty()) {
            None => PathBuf::from(DEFAULT_BIN),
            // The child runs in a temp dir, so a relative path is resolved here first.
            Some(bin) if Path::new(&bin).components().count() > 1 => std::path::absolute(&bin)
                .map_err(|err| format!("WAYPOINT_CLAUDE_BIN is not a usable path: {err}"))?,
            Some(bin) => PathBuf::from(bin),
        };
        Ok(Self::new(bin, timeout))
    }

    /// Sends `req.prompt` and returns the text reply.
    pub async fn call(
        &self,
        pool: &PgPool,
        req: &ClaudeRequest,
    ) -> Result<ClaudeResponse, ClaudeError> {
        self.run(pool, req, None, |_| Ok(())).await
    }

    /// Sends `req.prompt` with `--json-schema` and deserializes the structured output
    /// into `T`. A missing or mismatching output is logged and returned as
    /// [`ClaudeError::SchemaMismatch`].
    pub async fn call_structured<T: DeserializeOwned>(
        &self,
        pool: &PgPool,
        req: &ClaudeRequest,
        schema: &Value,
    ) -> Result<(T, ClaudeResponse), ClaudeError> {
        let mut typed = None;
        let response = self
            .run(pool, req, Some(schema), |structured| {
                let value = structured.ok_or_else(|| ClaudeError::SchemaMismatch {
                    message: "the result has no structured_output".into(),
                })?;
                typed = Some(
                    T::deserialize(value).map_err(|err| ClaudeError::SchemaMismatch {
                        message: err.to_string(),
                    })?,
                );
                Ok(())
            })
            .await?;
        let typed = typed.ok_or_else(|| ClaudeError::SchemaMismatch {
            message: "the result has no structured_output".into(),
        })?;
        Ok((typed, response))
    }

    async fn run(
        &self,
        pool: &PgPool,
        req: &ClaudeRequest,
        schema: Option<&Value>,
        check: impl FnOnce(Option<&Value>) -> Result<(), ClaudeError>,
    ) -> Result<ClaudeResponse, ClaudeError> {
        let started = Instant::now();
        let (meta, outcome) = self.execute(req, schema).await;
        let outcome = outcome.and_then(|done| check(done.structured.as_ref()).map(|()| done));
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let error_message = outcome.as_ref().err().map(ToString::to_string);
        let entry = log::Entry {
            purpose: &req.purpose,
            model: meta.model.as_deref(),
            usage: &meta.usage,
            duration_ms,
            cost_usd: meta.cost_usd,
            status: outcome.as_ref().map_or_else(ClaudeError::kind, |_| "ok"),
            error_message: error_message
                .as_deref()
                .map(|message| head(message, MAX_ERROR_BYTES)),
        };
        let log_id = match log::record(pool, &entry).await {
            Ok(id) => Some(id),
            Err(err) => {
                rocket::warn!("claude: cannot log the {:?} call: {err}", req.purpose);
                None
            }
        };

        outcome.map(|done| ClaudeResponse {
            text: done.text,
            structured: done.structured,
            model: meta.model,
            usage: meta.usage,
            duration_ms,
            cost_usd: meta.cost_usd,
            log_id,
        })
    }

    /// Spawns the CLI in a fresh temp dir, enforces the timeout and removes the dir.
    async fn execute(&self, req: &ClaudeRequest, schema: Option<&Value>) -> Outcome {
        let dir = match tempfile::Builder::new()
            .prefix("waypoint-claude-")
            .tempdir()
        {
            Ok(dir) => dir,
            Err(err) => {
                return process_error(format!("cannot create a temp dir for claude: {err}"));
            }
        };
        let mut command = self.command(req.model.as_deref(), schema, dir.path());
        // Dropping the future on timeout drops the child, and `kill_on_drop` kills it.
        let outcome = tokio::time::timeout(self.timeout, self.drive(&mut command, &req.prompt))
            .await
            .unwrap_or_else(|_| {
                (
                    Meta::default(),
                    Err(ClaudeError::Timeout {
                        after: self.timeout,
                    }),
                )
            });
        if let Err(err) = dir.close() {
            rocket::warn!("claude: cannot remove the call's temp dir: {err}");
        }
        outcome
    }

    fn command(&self, model: Option<&str>, schema: Option<&Value>, cwd: &Path) -> Command {
        let mut command = Command::new(&self.bin);
        command
            .args(args::build(model, schema))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group, so `GroupKill` can take down anything it starts.
            .process_group(0)
            .kill_on_drop(true);
        for key in scrubbed_env(std::env::vars_os().map(|(key, _)| key)) {
            command.env_remove(key);
        }
        command
    }

    /// Writes the prompt, reads stdout and stderr concurrently, and waits for exit.
    async fn drive(&self, command: &mut Command, prompt: &str) -> Outcome {
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => return process_error(self.spawn_error(&err)),
        };
        // Declared after `child`, so it drops first: on return, on the timeout and when
        // the caller drops this future.
        let _group = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .map(GroupKill);
        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return process_error("claude's stdio was not piped".into());
        };

        let input = format!("{}\n", user_message(prompt));
        let write = async move {
            stdin.write_all(input.as_bytes()).await?;
            stdin.shutdown().await
            // `stdin` is dropped here, closing the pipe so the CLI sees EOF.
        };
        let kill = &mut child;
        let read_stdout = async move {
            let mut lines = BufReader::new(stdout.take(MAX_STDOUT_BYTES + 1)).lines();
            let mut collected = Vec::new();
            let read = async {
                while let Some(line) = lines.next_line().await? {
                    collected.push(line);
                }
                Ok::<_, io::Error>(())
            };
            if let Err(err) = read.await {
                return Err(ClaudeError::Process {
                    message: format!("cannot read claude's output: {err}"),
                });
            }
            if lines.into_inner().into_inner().limit() == 0 {
                // Kill it so stderr closes and the join below doesn't wait for the timeout.
                let _ = kill.start_kill();
                return Err(ClaudeError::Protocol {
                    message: format!("claude wrote more than {MAX_STDOUT_BYTES} bytes"),
                });
            }
            Ok(collected)
        };
        let (written, lines, stderr_tail) = tokio::join!(write, read_stdout, tail(stderr));
        let status = child.wait().await;

        let lines = match lines {
            Ok(lines) => lines,
            Err(err) => return (Meta::default(), Err(err)),
        };
        let parsed = match stream::parse(lines.iter().map(String::as_str)) {
            Ok(parsed) => parsed,
            Err(err) => return (Meta::default(), Err(err)),
        };
        let meta = Meta {
            model: parsed.model.clone(),
            usage: parsed.usage.clone(),
            cost_usd: parsed.cost_usd,
        };
        if let Some(outcome) = parsed.outcome() {
            return (meta, outcome);
        }

        let status = match status {
            Ok(status) => status.to_string(),
            Err(err) => format!("unknown status ({err})"),
        };
        let mut message = format!("claude exited ({status}) without a result");
        if let Err(err) = written.as_ref() {
            message.push_str(&format!("; writing the prompt failed: {err}"));
        }
        if !stderr_tail.trim().is_empty() {
            message.push_str(&format!("; stderr: {}", stderr_tail.trim()));
        }
        (meta, Err(ClaudeError::Process { message }))
    }

    fn spawn_error(&self, err: &io::Error) -> String {
        if err.kind() == io::ErrorKind::NotFound && self.bin == Path::new(DEFAULT_BIN) {
            format!(
                "the `claude` CLI was not found on PATH ({err}); install Claude Code and log in with your Claude subscription"
            )
        } else {
            format!("cannot start {}: {err}", self.bin.display())
        }
    }
}

/// Sends `SIGKILL` to a call's whole process group when dropped, so grandchildren (a
/// shell's background job, an MCP server the CLI started) never outlive the call.
/// `kill_on_drop` stays as a backstop for the direct child.
///
/// Accepted race: once `wait()` has reaped the group leader, the group id could in theory
/// be reused before this guard drops. The window is the few microseconds between `wait()`
/// returning and the drop.
struct GroupKill(i32);

impl Drop for GroupKill {
    fn drop(&mut self) {
        // SAFETY: `killpg` only sends a signal; it touches no memory owned by this process.
        // The group was created for this call by `process_group(0)`, and an already-gone
        // group (`ESRCH`) is ignored on purpose.
        let _ = unsafe { libc::killpg(self.0, libc::SIGKILL) };
    }
}

/// The variables to remove from the child, given the names in the API's environment.
fn scrubbed_env(present: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut keys: Vec<OsString> = SCRUBBED_ENV.iter().map(OsString::from).collect();
    keys.extend(present.into_iter().filter(|key| {
        key.to_str().is_some_and(|key| {
            SCRUBBED_ENV_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
    }));
    keys
}

fn process_error(message: String) -> Outcome {
    (Meta::default(), Err(ClaudeError::Process { message }))
}

/// The single stream-json user message that carries the prompt.
fn user_message(prompt: &str) -> Value {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": prompt}],
        },
    })
}

/// Reads `reader` to EOF and keeps only its last [`MAX_ERROR_BYTES`] bytes.
async fn tail(mut reader: impl AsyncRead + Unpin) -> String {
    let mut kept = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > MAX_ERROR_BYTES {
                    kept.drain(..kept.len() - MAX_ERROR_BYTES);
                }
            }
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// The longest prefix of `text` that fits in `max` bytes without splitting a character.
pub(crate) fn head(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn command_scrubs_api_key_env() {
        let runner = ClaudeRunner::new("claude", DEFAULT_TIMEOUT);
        let cwd = Path::new("/tmp/waypoint-test");
        let command = runner.command(None, None, cwd);
        let std = command.as_std();

        let envs: Vec<_> = std.get_envs().collect();
        for key in SCRUBBED_ENV {
            assert!(
                envs.contains(&(OsStr::new(key), None)),
                "{key} is removed: {envs:?}"
            );
        }
        assert_eq!(std.get_current_dir(), Some(cwd));
        assert_eq!(std.get_program(), "claude");
        assert!(!std.get_args().any(|arg| arg == "--bare"));
    }

    #[test]
    fn scrubbed_env_covers_billing_switches_and_waypoint_settings() {
        let present = [
            "PATH",
            "HOME",
            "POSTGRES_PASSWORD",
            "WAYPOINT_API_PORT",
            "PGHOST",
        ];
        let keys = scrubbed_env(present.map(OsString::from));
        for key in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
            "DATABASE_URL",
            "POSTGRES_PASSWORD",
            "WAYPOINT_API_PORT",
        ] {
            assert!(keys.contains(&OsString::from(key)), "{key} is removed");
        }
        for kept in ["PATH", "HOME", "PGHOST"] {
            assert!(!keys.contains(&OsString::from(kept)), "{kept} is kept");
        }
    }

    #[test]
    fn timeout_defaults_to_300s_and_rejects_garbage() {
        assert_eq!(parse_timeout(None), Ok(Duration::from_secs(300)));
        assert_eq!(parse_timeout(Some(" ")), Ok(Duration::from_secs(300)));
        assert_eq!(parse_timeout(Some("45")), Ok(Duration::from_secs(45)));
        for bad in ["nope", "0", "-3"] {
            let err = parse_timeout(Some(bad)).unwrap_err();
            assert!(err.contains("WAYPOINT_CLAUDE_TIMEOUT_SECS"), "{err}");
        }
    }

    #[test]
    fn head_and_user_message() {
        assert_eq!(head("abc", 8), "abc");
        assert_eq!(head("aé", 2), "a");
        assert_eq!(
            user_message("hi"),
            json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "hi"}]}})
        );
    }
}
