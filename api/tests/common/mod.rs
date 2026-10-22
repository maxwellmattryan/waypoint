//! Test helpers shared by the integration tests that drive the Claude runner, the job
//! queue and the HTTP API. Each test crate uses only some of them.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rocket::{Build, Rocket};
use serde_json::Value;
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::events::{Event, Subscriber};
use waypoint_api::jobs;

/// The `Host` header every HTTP test sends; the API rejects requests without one.
pub const HOST: &str = "127.0.0.1";

/// A runner pointed at `fake-claude.sh` in one mode, with the directory the fake records
/// its argv, cwd, cwd listing, env and stdin into.
pub struct Fake {
    pub runner: ClaudeRunner,
    pub record: PathBuf,
    _dir: TempDir,
}

/// Builds a per-test wrapper script that runs the fake in `mode`, so no test mutates the
/// process environment.
pub fn fake(mode: &str, timeout: Duration) -> Fake {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = dir.path().join("record");
    fs::create_dir(&record).expect("record dir");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude/fake-claude.sh");
    let wrapper = dir.path().join("claude");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' '{mode}' '{}' \"$@\"\n",
            script.display(),
            record.display()
        ),
    )
    .expect("write wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("chmod wrapper");
    Fake {
        runner: ClaudeRunner::new(wrapper, timeout),
        record,
        _dir: dir,
    }
}

/// The API with Rocket's default test config.
pub fn app(pool: PgPool, runner: ClaudeRunner, config: jobs::Config) -> Rocket<Build> {
    waypoint_api::rocket(
        rocket::Config::figment(),
        pool,
        runner,
        config,
        waypoint_api::google::Config::default(),
    )
}

/// Queue settings that keep tests fast: 50ms polls and retries, a 60s usage-limit pause.
pub fn fast_config() -> jobs::Config {
    jobs::Config {
        concurrency: 2,
        poll_interval: Duration::from_millis(50),
        retry_base: Duration::from_millis(50),
        retry_cap: Duration::from_secs(1),
        usage_limit_pause: Duration::from_secs(60),
        max_attempts: 3,
    }
}

/// Collects events until `done` holds for the last one, or `timeout` passes. Returns
/// everything received either way.
pub async fn events(
    subscriber: &mut Subscriber,
    timeout: Duration,
    mut done: impl FnMut(&Event) -> bool,
) -> Vec<Event> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(timeout, async {
        while let Some(event) = subscriber.next().await {
            let finished = done(&event);
            seen.push(event);
            if finished {
                break;
            }
        }
    })
    .await;
    seen
}

/// The next SSE message as `(event, data)`, skipping heartbeat comments.
pub async fn next_message(reader: &mut (impl AsyncBufRead + Unpin)) -> (String, Value) {
    let mut event = None;
    let mut data = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await.expect("read the stream");
        assert!(read > 0, "the stream ended");
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if let (Some(event), Some(data)) = (event.take(), data.take()) {
                return (event, data);
            }
        } else if let Some(name) = line.strip_prefix("event:") {
            event = Some(name.trim().to_owned());
        } else if let Some(json) = line.strip_prefix("data:") {
            data = Some(serde_json::from_str(json.trim()).expect("data is JSON"));
        }
    }
}
