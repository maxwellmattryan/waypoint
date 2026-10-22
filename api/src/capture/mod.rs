//! `POST /captures`: read a pasted job URL and return the fields for the add form.
//!
//! Tiers run in order and the first one with a title wins: the page's JSON-LD
//! `JobPosting` ([`jsonld`]), then the board API for a Greenhouse, Lever or Ashby URL
//! ([`ats`]), both inside the request, then a `capture_extract` job that asks Claude to
//! read the page's visible text ([`llm`]). Nothing is written to `applications`; the user
//! confirms the fields and the add form creates the row. See `docs/capture.md`.

pub mod ats;
pub mod fetch;
pub mod fields;
pub mod jsonld;
pub mod llm;
pub mod text;

use std::net::SocketAddr;
use std::time::Duration;

use reqwest::Url;
use rocket::State;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{self, Responder};
use rocket::serde::json::{self, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;

use self::fetch::FetchError;
use self::fields::Extracted;
use crate::applications::{
    ApiError, FieldError, JsonContent, MAX_URL, REQUIRED, ValidationErrors, internal, is_web_url,
    parsed,
};
use crate::events::EventBus;
use crate::host::{LocalHost, NotCrossSite};
use crate::jobs::{self, Wake};

/// Most characters of page text sent to Claude.
pub const MAX_TEXT: usize = 30_000;
const BAD_URL: &str = "Must be an http or https URL";
const CREDENTIALS: &str = "Must not contain a username or password";
const REDIRECT_REFUSED: &str =
    "The link redirects to a private network address, so Waypoint didn't follow it.";
const USER_AGENT: &str = "Mozilla/5.0 (compatible; Waypoint/0.1; personal job tracker)";

/// One job board: the hosts its posting URLs live on and its API base.
#[derive(Debug, Clone, PartialEq)]
pub struct Board {
    /// Each matches a URL's `host` or `host:port`, case-insensitively.
    pub hosts: Vec<String>,
    pub api: String,
}

impl Board {
    fn new(hosts: &[&str], api: &str) -> Self {
        Self {
            hosts: hosts.iter().map(|host| (*host).to_string()).collect(),
            api: api.into(),
        }
    }
}

/// Capture settings. `main.rs` always uses the default; tests point the boards at
/// wiremock and shorten the limits.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// One deadline for the whole fetch: every redirect hop and the body.
    pub timeout: Duration,
    /// Most body bytes read from a page or an API.
    pub max_bytes: u64,
    pub greenhouse: Board,
    pub lever: Board,
    pub ashby: Board,
    /// Test-only: socket addresses the redirect guard treats as public, so a test can
    /// stand up a "public" origin on loopback.
    pub treat_as_public: Vec<SocketAddr>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
            max_bytes: 5 * 1024 * 1024,
            greenhouse: Board::new(
                &["boards.greenhouse.io", "job-boards.greenhouse.io"],
                "https://boards-api.greenhouse.io",
            ),
            lever: Board::new(&["jobs.lever.co"], "https://api.lever.co"),
            ashby: Board::new(&["jobs.ashbyhq.com"], "https://api.ashbyhq.com"),
            treat_as_public: Vec::new(),
        }
    }
}

/// Managed state: the settings and the one HTTP client every capture uses. Redirects are
/// followed by [`fetch`], never by reqwest.
#[derive(Debug, Clone)]
pub struct Capturer {
    pub config: Config,
    client: reqwest::Client,
}

impl Capturer {
    pub fn new(config: Config) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            // A proxy would make every `remote_addr` the proxy's, which blinds the
            // redirect guard, so proxy environment variables are ignored.
            .no_proxy()
            .user_agent(USER_AGENT)
            .build()
            .expect("the HTTP client builds");
        Self { config, client }
    }
}

/// The `POST /captures` body.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct CaptureRequest {
    /// The job posting's absolute http or https URL. Required.
    #[serde(default)]
    #[schema(required = true)]
    pub url: Option<String>,
}

/// How the fields were found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMethod {
    /// The page's JSON-LD `JobPosting`.
    JsonLd,
    Greenhouse,
    Lever,
    Ashby,
    /// Claude read the page text.
    Llm,
}

/// The fields for the add form, normalised: trimmed, blank as `null`, at most 200
/// characters each.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CapturedFields {
    #[schema(required = true)]
    pub company: Option<String>,
    #[schema(required = true)]
    pub title: Option<String>,
    /// The trimmed URL the user submitted.
    pub url: String,
    #[schema(required = true)]
    pub location: Option<String>,
    #[schema(required = true)]
    pub comp: Option<String>,
}

/// A capture: the 200 body, and a succeeded `capture_extract` job's `result`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Capture {
    pub method: CaptureMethod,
    pub fields: CapturedFields,
}

/// The 202 body: Claude will read the page in this job.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct CaptureQueued {
    pub job_id: i64,
}

/// Why the page couldn't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureFailureReason {
    Timeout,
    Unreachable,
    HttpStatus,
    NotHtml,
    TooLarge,
    EmptyPage,
}

/// The 502 body. It never includes page content.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct CaptureFailure {
    pub reason: CaptureFailureReason,
    /// A sentence for the user.
    pub message: String,
}

impl CaptureFailure {
    pub fn new(reason: CaptureFailureReason) -> Self {
        let message = match reason {
            CaptureFailureReason::Timeout => "The page took too long to answer.",
            CaptureFailureReason::Unreachable => "Couldn't connect to the site.",
            CaptureFailureReason::HttpStatus => "The site answered with an error.",
            CaptureFailureReason::NotHtml => "The link isn't a web page.",
            CaptureFailureReason::TooLarge => "The page is too large to read.",
            CaptureFailureReason::EmptyPage => "The page has no readable text.",
        };
        Self {
            reason,
            message: message.into(),
        }
    }

    pub fn http_status(code: u16) -> Self {
        Self {
            reason: CaptureFailureReason::HttpStatus,
            message: format!("The site answered with HTTP {code}."),
        }
    }
}

/// What `POST /captures` answers when the URL was valid.
#[derive(Debug)]
pub enum CaptureResponse {
    Captured(Capture),
    Queued(CaptureQueued),
    Unreadable(CaptureFailure),
}

impl<'r> Responder<'r, 'static> for CaptureResponse {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        match self {
            Self::Captured(capture) => Json(capture).respond_to(req),
            Self::Queued(queued) => (Status::Accepted, Json(queued)).respond_to(req),
            Self::Unreadable(failure) => (Status::BadGateway, Json(failure)).respond_to(req),
        }
    }
}

fn url_error(message: &str) -> ApiError {
    ApiError::Invalid(ValidationErrors {
        errors: vec![FieldError {
            field: "url".into(),
            message: message.into(),
        }],
    })
}

/// Trims and checks the URL; nothing is fetched when this fails.
fn validate(url: Option<&str>) -> Result<(String, Url), ApiError> {
    let mut errors = Vec::new();
    let trimmed = crate::applications::text(&mut errors, "url", url, MAX_URL)
        .ok_or_else(|| url_error(REQUIRED))?;
    if !errors.is_empty() {
        return Err(ApiError::Invalid(ValidationErrors { errors }));
    }
    if !is_web_url(&trimmed) {
        return Err(url_error(BAD_URL));
    }
    let parsed = Url::parse(&trimmed).map_err(|_| url_error(BAD_URL))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(url_error(CREDENTIALS));
    }
    Ok((trimmed, parsed))
}

fn captured(method: CaptureMethod, url: &str, found: Extracted) -> CaptureResponse {
    CaptureResponse::Captured(Capture {
        method,
        fields: CapturedFields {
            company: found.company,
            title: found.title,
            url: url.to_string(),
            location: found.location,
            comp: found.comp,
        },
    })
}

/// Tier 2 for a detected board; a failure is logged and gives `None`.
async fn board(
    capturer: &Capturer,
    host: &str,
    url: &str,
    ats: Option<&ats::Ats>,
) -> Option<CaptureResponse> {
    let ats = ats?;
    match ats::lookup(&capturer.client, &capturer.config, ats).await {
        Ok(found) => {
            let method = match ats {
                ats::Ats::Greenhouse { .. } => CaptureMethod::Greenhouse,
                ats::Ats::Lever { .. } => CaptureMethod::Lever,
                ats::Ats::Ashby { .. } => CaptureMethod::Ashby,
            };
            Some(captured(method, url, found))
        }
        Err(reason) => {
            rocket::warn!("captures: {} API for {host} failed: {reason}", ats.name());
            None
        }
    }
}

/// Reads a job posting URL and returns its fields for the add form, or queues Claude to
/// read the page.
#[utoipa::path(
    post,
    path = "/captures",
    tag = "captures",
    operation_id = "capture_url",
    request_body = CaptureRequest,
    responses(
        (status = 200, description = "The fields, found in the page's JSON-LD or a job board's API", body = Capture),
        (status = 202, description = "No structured data; Claude reads the page in this job, whose `result` is a `Capture`", body = CaptureQueued),
        (status = 400, description = "The body is not valid JSON for this route", body = ValidationErrors),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 415, description = "The body is not `application/json`"),
        (status = 422, description = "The URL is invalid, or it redirects from a public address to a private one", body = ValidationErrors),
        (status = 500, description = "The extraction job could not be queued"),
        (status = 502, description = "The page could not be read", body = CaptureFailure),
    )
)]
#[rocket::post("/captures", data = "<body>")]
#[allow(clippy::too_many_arguments)]
pub async fn capture(
    _host: LocalHost,
    _site: NotCrossSite,
    _json: JsonContent,
    capturer: &State<Capturer>,
    pool: &State<PgPool>,
    bus: &State<EventBus>,
    wake: &State<Wake>,
    body: Result<Json<CaptureRequest>, json::Error<'_>>,
) -> Result<CaptureResponse, ApiError> {
    let request = parsed(body)?;
    let (url, parsed_url) = validate(request.url.as_deref())?;
    // Only the host is logged: a full URL can carry tokens.
    let host = parsed_url.host_str().unwrap_or_default().to_string();
    let detected = ats::detect(&capturer.config, &parsed_url);

    let page = match fetch::fetch_page(&capturer.client, &capturer.config, parsed_url).await {
        Ok(page) => page,
        Err(FetchError::RedirectRefused { host: refused }) => {
            rocket::warn!("captures: refused a redirect to {refused}");
            return Err(url_error(REDIRECT_REFUSED));
        }
        Err(FetchError::Failed(failure)) => {
            rocket::warn!(
                "captures: cannot read the page at {host}: {:?}",
                failure.reason
            );
            let rescued = board(capturer, &host, &url, detected.as_ref()).await;
            return Ok(rescued.unwrap_or(CaptureResponse::Unreadable(failure)));
        }
    };

    if let Some(found) = jsonld::extract(&page.body) {
        return Ok(captured(CaptureMethod::JsonLd, &url, found));
    }
    if let Some(found) = board(capturer, &host, &url, detected.as_ref()).await {
        return Ok(found);
    }
    let text = text::visible_text(&page.body, MAX_TEXT);
    if text.is_empty() {
        rocket::warn!("captures: the page at {host} has no readable text");
        return Ok(CaptureResponse::Unreadable(CaptureFailure::new(
            CaptureFailureReason::EmptyPage,
        )));
    }
    let job = jobs::enqueue(
        pool,
        bus,
        wake,
        jobs::CAPTURE_EXTRACT,
        json!({"url": url, "text": text}),
    )
    .await
    .map_err(internal("queue a capture"))?;
    Ok(CaptureResponse::Queued(CaptureQueued { job_id: job.id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_messages() {
        assert_eq!(
            CaptureFailure::http_status(404).message,
            "The site answered with HTTP 404."
        );
        assert_eq!(
            CaptureFailure::new(CaptureFailureReason::NotHtml).message,
            "The link isn't a web page."
        );
    }
}
