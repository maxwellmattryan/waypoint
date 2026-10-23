//! Debug-build-only routes for checking integrations by hand. Release builds don't mount
//! them.

use rocket::State;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::serde::json::Json;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::claude::{ClaudeError, ClaudeRequest, ClaudeRunner};
use crate::email::{self, IngestError, NewEmail, Source, VerdictKind};
use crate::events::EventBus;
use crate::host::LocalHost;
use crate::jobs::{self, Wake};

const PING_PROMPT: &str = "Reply with exactly the word: pong";
/// Header every `/dev` request must carry as `1`. A custom header makes a browser
/// preflight the request, and the API answers no CORS preflight, so other sites can't
/// trigger these routes.
const DEV_HEADER: &str = "X-Waypoint-Dev";

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![claude_ping, jobs_test_prompt, emails_import_fixtures]
}

/// The synthetic email corpus `POST /dev/emails/import-fixtures` reads.
const EMAIL_FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/emails");

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

/// `POST /dev/emails/import-fixtures`: ingests every `*.eml` under `api/fixtures/emails/`
/// in file-name order, with `source = 'fixture'` and the file stem as the provider message
/// id. A file that can't be read or parsed is listed in `failed` and the rest still
/// import; a database error answers 500. `threads` counts the distinct threads the
/// imported emails belong to.
#[rocket::post("/dev/emails/import-fixtures")]
async fn emails_import_fixtures(
    _host: LocalHost,
    _dev: DevOnly,
    pool: &State<PgPool>,
) -> Result<Json<Value>, Status> {
    let fail = |what: &str, err: &dyn std::fmt::Display| {
        rocket::warn!("dev: cannot import email fixtures: {what}: {err}");
        Status::InternalServerError
    };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(EMAIL_FIXTURES)
        .map_err(|err| fail("read the fixture directory", &err))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "eml"))
        .collect();
    files.sort();
    let known = email::known_companies(pool)
        .await
        .map_err(|err| fail("load known companies", &err))?;

    let (mut inserted, mut updated, mut candidates, mut skipped) = (0, 0, 0, 0);
    let mut threads = std::collections::BTreeSet::new();
    let mut failed = Vec::new();
    for path in &files {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(err) => {
                failed.push(json!({"file": file, "error": err.to_string()}));
                continue;
            }
        };
        let new = NewEmail {
            source: Source::Fixture,
            provider_message_id: stem,
            provider_thread_id: None,
            labels: Vec::new(),
            raw,
        };
        match email::ingest(pool, new, &known).await {
            Ok(done) => {
                if done.inserted {
                    inserted += 1;
                } else {
                    updated += 1;
                }
                match done.verdict {
                    VerdictKind::Candidate => candidates += 1,
                    VerdictKind::Skip => skipped += 1,
                }
                threads.insert(done.thread_id);
            }
            Err(IngestError::Parse(err)) => {
                failed.push(json!({"file": file, "error": err.to_string()}));
            }
            Err(IngestError::Db(err)) => return Err(fail(&file, &err)),
        }
    }

    Ok(Json(json!({
        "files": files.len(),
        "inserted": inserted,
        "updated": updated,
        "candidates": candidates,
        "skipped": skipped,
        "threads": threads.len(),
        "failed": failed,
    })))
}
