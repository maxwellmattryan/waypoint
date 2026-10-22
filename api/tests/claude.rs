//! The Claude runner against `fake-claude.sh`; the real CLI is never called here.

mod common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use waypoint_api::claude::{ClaudeError, ClaudeRequest, ClaudeRunner};

const TIMEOUT: Duration = Duration::from_secs(10);

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

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct Row {
    id: i64,
    purpose: String,
    status: String,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    duration_ms: i64,
    cost_usd: Option<f64>,
    error_message: Option<String>,
}

async fn rows(pool: &PgPool) -> Vec<Row> {
    sqlx::query_as(
        "SELECT id, purpose, status, model, input_tokens, output_tokens, \
         cache_creation_input_tokens, cache_read_input_tokens, duration_ms, cost_usd, \
         error_message FROM claude_calls ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("read claude_calls")
}

#[sqlx::test]
async fn fake_binary_receives_isolated_argv_cwd_and_stdin(pool: PgPool) {
    let fake = common::fake("success", TIMEOUT);
    let prompt = "unique prompt text 4711";
    fake.runner
        .call(&pool, &request("test.argv", prompt))
        .await
        .expect("call succeeds");

    let argv: Vec<String> = recorded(&fake.record, "argv")
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        argv,
        [
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
        ]
    );
    assert!(!argv.iter().any(|arg| arg == "--bare"));
    assert!(!argv.iter().any(|arg| arg.contains(prompt)));

    let cwd = recorded(&fake.record, "cwd");
    let cwd = Path::new(cwd.trim());
    let api_cwd = std::env::current_dir().expect("cwd");
    assert_ne!(cwd, api_cwd);
    assert!(!cwd.starts_with(&api_cwd), "{cwd:?} is inside {api_cwd:?}");
    assert_eq!(
        recorded(&fake.record, "ls"),
        "",
        "the temp dir starts empty"
    );
    assert!(!cwd.exists(), "{cwd:?} is removed after the call");
    assert_eq!(recorded(&fake.record, "env_api_key").trim(), "unset");

    let stdin = recorded(&fake.record, "stdin");
    assert_eq!(stdin.lines().count(), 1, "one stream-json line: {stdin:?}");
    let message: Value = serde_json::from_str(&stdin).expect("stdin is JSON");
    assert_eq!(
        message,
        json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": prompt}]}})
    );
}

#[sqlx::test]
async fn success_call_returns_text_and_logs_row(pool: PgPool) {
    let fake = common::fake("success", TIMEOUT);
    let res = fake
        .runner
        .call(&pool, &request("test.success", "ping"))
        .await
        .expect("call succeeds");

    assert_eq!(res.text, "pong");
    assert_eq!(res.model.as_deref(), Some("claude-haiku-5-5"));
    assert_eq!(res.usage.input_tokens, Some(2));
    assert_eq!(res.usage.output_tokens, Some(4));
    assert_eq!(res.usage.cache_creation_input_tokens, Some(3327));
    assert_eq!(res.usage.cache_read_input_tokens, Some(649));
    assert_eq!(res.cost_usd, Some(0.00067409));
    assert_eq!(res.structured, None);

    let rows = rows(&pool).await;
    assert_eq!(
        rows,
        [Row {
            id: res.log_id.expect("logged"),
            purpose: "test.success".into(),
            status: "ok".into(),
            model: Some("claude-haiku-5-5".into()),
            input_tokens: Some(2),
            output_tokens: Some(4),
            cache_creation_input_tokens: Some(3327),
            cache_read_input_tokens: Some(649),
            duration_ms: i64::try_from(res.duration_ms).unwrap(),
            cost_usd: Some(0.00067409),
            error_message: None,
        }]
    );
}

#[sqlx::test]
async fn usage_limit_is_typed_and_logged(pool: PgPool) {
    let fake = common::fake("usage_limit", TIMEOUT);
    let err = fake
        .runner
        .call(&pool, &request("test.limit", "ping"))
        .await
        .expect_err("usage limit");

    let ClaudeError::UsageLimit { message } = &err else {
        panic!("expected UsageLimit, got {err:?}");
    };
    assert!(message.contains("hit your limit"), "{message}");

    let rows = rows(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "usage_limit");
    assert_eq!(rows[0].purpose, "test.limit");
    assert!(
        rows[0]
            .error_message
            .as_deref()
            .is_some_and(|m| m.contains("hit your limit"))
    );
}

#[derive(Debug, Deserialize, PartialEq)]
struct Answer {
    word: String,
    n: i64,
}

#[sqlx::test]
async fn structured_output_is_validated(pool: PgPool) {
    let schema = json!({
        "type": "object",
        "properties": {"word": {"type": "string"}, "n": {"type": "integer"}},
        "required": ["word", "n"],
    });

    let fake = common::fake("structured", TIMEOUT);
    let (answer, res) = fake
        .runner
        .call_structured::<Answer>(&pool, &request("test.structured", "ping"), &schema)
        .await
        .expect("structured call succeeds");
    assert_eq!(
        answer,
        Answer {
            word: "pong".into(),
            n: 2
        }
    );
    assert_eq!(res.structured, Some(json!({"word": "pong", "n": 2})));
    let argv = recorded(&fake.record, "argv");
    let argv: Vec<&str> = argv.lines().collect();
    let at = argv
        .iter()
        .position(|arg| *arg == "--json-schema")
        .expect("--json-schema passed");
    let sent: Value = serde_json::from_str(argv[at + 1]).expect("schema arg is JSON");
    assert_eq!(sent, schema);

    for mode in ["structured_wrong", "success"] {
        let fake = common::fake(mode, TIMEOUT);
        let err = fake
            .runner
            .call_structured::<Answer>(&pool, &request("test.structured", "ping"), &schema)
            .await
            .expect_err("schema mismatch");
        assert!(
            matches!(err, ClaudeError::SchemaMismatch { .. }),
            "{mode}: {err:?}"
        );
    }

    let statuses: Vec<String> = rows(&pool)
        .await
        .into_iter()
        .map(|row| row.status)
        .collect();
    assert_eq!(statuses, ["ok", "schema_mismatch", "schema_mismatch"]);
}

#[sqlx::test]
async fn failures_are_typed_and_logged(pool: PgPool) {
    let req = request("test.failure", "ping");

    let hang = common::fake("hang", Duration::from_secs(1));
    let err = hang.runner.call(&pool, &req).await.expect_err("timeout");
    assert_eq!(
        err,
        ClaudeError::Timeout {
            after: Duration::from_secs(1)
        }
    );

    let exit1 = common::fake("exit1", TIMEOUT);
    let err = exit1.runner.call(&pool, &req).await.expect_err("exit 1");
    let ClaudeError::Process { message } = &err else {
        panic!("expected Process, got {err:?}");
    };
    assert!(message.contains("something broke"), "{message}");

    let garbage = common::fake("garbage", TIMEOUT);
    let err = garbage.runner.call(&pool, &req).await.expect_err("garbage");
    assert!(matches!(err, ClaudeError::Protocol { .. }), "{err:?}");

    let flood = common::fake("flood", TIMEOUT);
    let err = flood.runner.call(&pool, &req).await.expect_err("flood");
    let ClaudeError::Protocol { message } = &err else {
        panic!("expected Protocol, got {err:?}");
    };
    assert!(message.contains("more than"), "{message}");

    let cli = common::fake("cli_error", TIMEOUT);
    let err = cli.runner.call(&pool, &req).await.expect_err("cli error");
    let ClaudeError::Cli { subtype, message } = &err else {
        panic!("expected Cli, got {err:?}");
    };
    assert_eq!(subtype, "error_max_turns");
    assert!(message.contains("maximum number of turns"), "{message}");

    let missing = ClaudeRunner::new("/nonexistent/waypoint/claude", TIMEOUT);
    let err = missing.call(&pool, &req).await.expect_err("missing binary");
    assert!(matches!(err, ClaudeError::Process { .. }), "{err:?}");

    let rows = rows(&pool).await;
    let statuses: Vec<&str> = rows.iter().map(|row| row.status.as_str()).collect();
    assert_eq!(
        statuses,
        [
            "timeout",
            "process_failed",
            "protocol_error",
            "protocol_error",
            "cli_error",
            "process_failed"
        ]
    );
    assert!(rows.iter().all(|row| row.error_message.is_some()));
    assert!(rows[0].duration_ms >= 1000, "{:?}", rows[0]);
    assert_eq!(rows[4].model.as_deref(), Some("claude-haiku-5-5"));
}

/// The pid of the background `sleep` that `hang_tree` starts, once the fake has written it.
async fn grandchild_pid(record: &Path) -> i32 {
    let file = record.join("grandchild");
    for _ in 0..200 {
        if let Some(pid) = fs::read_to_string(&file)
            .ok()
            .and_then(|raw| raw.trim().parse::<i32>().ok())
        {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the fake never wrote its grandchild's pid");
}

/// Whether `pid` is gone within 2s, polling every 50ms.
async fn dies_within_2s(pid: i32) -> bool {
    for _ in 0..40 {
        // SAFETY: signal 0 only checks that the process exists; nothing is delivered.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[sqlx::test]
async fn timeout_and_cancel_kill_the_process_group(pool: PgPool) {
    let req = request("test.group", "ping");

    let timed_out = common::fake("hang_tree", Duration::from_secs(1));
    let (result, pid) = tokio::join!(
        timed_out.runner.call(&pool, &req),
        grandchild_pid(&timed_out.record)
    );
    let err = result.expect_err("timeout");
    assert!(matches!(err, ClaudeError::Timeout { .. }), "{err:?}");
    assert!(
        dies_within_2s(pid).await,
        "the grandchild outlived the timeout"
    );

    // The caller gives up after 1s, well inside the runner's own 10s timeout. The
    // grandchild must exist before the future is dropped, so wait for it too.
    let cancelled = common::fake("hang_tree", TIMEOUT);
    let call = tokio::time::timeout(Duration::from_secs(1), cancelled.runner.call(&pool, &req));
    let (dropped, pid) = tokio::join!(call, grandchild_pid(&cancelled.record));
    assert!(dropped.is_err(), "the call was still running when dropped");
    assert!(
        dies_within_2s(pid).await,
        "the grandchild outlived the dropped call"
    );
}
