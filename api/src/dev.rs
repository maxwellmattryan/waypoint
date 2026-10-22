//! Debug-build-only routes for checking integrations by hand. Release builds don't mount
//! them.

use rocket::State;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::serde::json::Json;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::claude::{ClaudeError, ClaudeRequest, ClaudeRunner};
use crate::events::EventBus;
use crate::host::LocalHost;
use crate::jobs::{self, Wake};

const PING_PROMPT: &str = "Reply with exactly the word: pong";
/// Header every `/dev` request must carry as `1`. A custom header makes a browser
/// preflight the request, and the API answers no CORS preflight, so other sites can't
/// trigger these routes.
const DEV_HEADER: &str = "X-Waypoint-Dev";

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![claude_ping, jobs_test_prompt]
}

/// Admits a request only if it carries `X-Waypoint-Dev: 1` and a browser, if one sent
/// it, sent it from the API's own origin.
struct DevOnly;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for DevOnly {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let headers = req.headers();
        let marked = headers.get_one(DEV_HEADER) == Some("1");
        // Browsers send `Sec-Fetch-Site`; curl doesn't. Another localhost port is
        // `same-site`, so only `same-origin` and `none` (typed by the user) pass.
        let foreign = headers.get("Sec-Fetch-Site").any(|site| {
            !site.eq_ignore_ascii_case("same-origin") && !site.eq_ignore_ascii_case("none")
        });
        if marked && !foreign {
            Outcome::Success(Self)
        } else {
            Outcome::Error((Status::Forbidden, ()))
        }
    }
}

/// `POST /dev/claude/ping`: one round trip through the Claude runner.
#[rocket::post("/dev/claude/ping")]
async fn claude_ping(
    _host: LocalHost,
    _dev: DevOnly,
    pool: &State<PgPool>,
    claude: &State<ClaudeRunner>,
) -> (Status, Json<Value>) {
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

/// `POST /dev/jobs/test-prompt`: enqueues a `test_prompt` job with the ping prompt and
/// answers `202 {"job_id"}`. Follow it on `/events` or at `/jobs/<id>`.
#[rocket::post("/dev/jobs/test-prompt")]
async fn jobs_test_prompt(
    _host: LocalHost,
    _dev: DevOnly,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
) -> Result<(Status, Json<Value>), Status> {
    let payload = json!({"prompt": PING_PROMPT});
    match jobs::enqueue(pool, bus, wake, jobs::TEST_PROMPT, payload).await {
        Ok(job) => Ok((Status::Accepted, Json(json!({"job_id": job.id})))),
        Err(err) => {
            rocket::warn!("dev: cannot enqueue a test_prompt job: {err}");
            Err(Status::InternalServerError)
        }
    }
}
