//! Debug-build-only routes for checking integrations by hand. Release builds don't mount
//! them.

use rocket::State;
use rocket::http::Status;
use rocket::serde::json::Json;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::claude::{ClaudeError, ClaudeRequest, ClaudeRunner};

const PING_PROMPT: &str = "Reply with exactly the word: pong";

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![claude_ping]
}

/// `POST /dev/claude/ping`: one round trip through the Claude runner.
#[rocket::post("/dev/claude/ping")]
async fn claude_ping(pool: &State<PgPool>, claude: &State<ClaudeRunner>) -> (Status, Json<Value>) {
    let req = ClaudeRequest {
        prompt: PING_PROMPT.into(),
        purpose: "dev.ping".into(),
        model: None,
    };
    match claude.call(pool, &req).await {
        Ok(res) => (
            Status::Ok,
            Json(json!({
                "text": res.text,
                "model": res.model,
                "input_tokens": res.usage.input_tokens,
                "output_tokens": res.usage.output_tokens,
                "cache_read_input_tokens": res.usage.cache_read_input_tokens,
                "cache_creation_input_tokens": res.usage.cache_creation_input_tokens,
                "duration_ms": res.duration_ms,
                "cost_usd": res.cost_usd,
                "log_id": res.log_id,
            })),
        ),
        Err(ClaudeError::UsageLimit { message }) => (
            Status::TooManyRequests,
            Json(json!({"error": "usage_limit", "message": message})),
        ),
        Err(err) => (
            Status::BadGateway,
            Json(json!({
                "error": "claude_failed",
                "kind": err.kind(),
                "message": err.to_string(),
            })),
        ),
    }
}
