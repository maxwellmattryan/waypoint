//! Google OAuth for Gmail (read-only) and Calendar (events): a loopback-redirect
//! authorization-code flow with PKCE and a single-use state, tokens sealed at rest with
//! [`crate::crypto`], and refresh under a row lock.
//!
//! Routes: `GET /auth/google/connect` starts the flow, `GET /auth/google/callback` is
//! where Google sends the browser back, `GET /auth/google` reports the status and
//! `DELETE /auth/google` disconnects. Later jobs get a token from [`access_token`].
//!
//! Tokens, codes, verifiers, the client secret and the key never reach a log line, an
//! error message or a plaintext column.

pub mod config;

use std::fmt;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rocket::State;
use rocket::http::Status;
use rocket::response::Redirect;
use rocket::response::content::RawHtml;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

pub use self::config::Config;
use self::config::Credentials;
use crate::crypto::{self, TokenKey};
use crate::host::{LocalHost, NotCrossSite};

/// Read the user's Gmail.
pub const GMAIL_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";
/// View and edit events on the user's calendars.
pub const CALENDAR_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";
/// Every scope Waypoint asks for; a grant missing any of them is refused.
pub const SCOPES: [&str; 2] = [GMAIL_READONLY, CALENDAR_EVENTS];

/// AAD for the sealed `google_accounts.refresh_token` column.
pub const REFRESH_TOKEN_AAD: &[u8] = b"google_accounts.refresh_token";
/// AAD for the sealed `google_accounts.access_token` column.
pub const ACCESS_TOKEN_AAD: &[u8] = b"google_accounts.access_token";

/// How long a started connect flow stays valid, in minutes.
const STATE_TTL_MINUTES: i32 = 10;
/// A stored access token is used only while it stays valid this much longer.
const REFRESH_MARGIN_SECS: i32 = 60;
/// Assumed lifetime when Google omits `expires_in`.
const DEFAULT_EXPIRES_IN: i64 = 3600;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether a Google account is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// No account is connected.
    Disconnected,
    /// Tokens are stored and usable.
    Connected,
    /// Google revoked or expired the grant, or the stored tokens no longer decrypt.
    /// Connect again.
    ReconnectRequired,
}

/// The `GET /auth/google` body.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GoogleStatus {
    /// Whether the client id, secret and token key are all set.
    pub configured: bool,
    pub state: ConnectionState,
    /// The granted scopes, sorted; empty when disconnected.
    pub scopes: Vec<String>,
    /// When the account was last connected; `null` when disconnected.
    #[schema(required = true)]
    pub connected_at: Option<DateTime<Utc>>,
}

/// The `DELETE /auth/google` body.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct GoogleDisconnect {
    /// Whether Google confirmed the revocation. When `false`, remove the grant by hand at
    /// <https://myaccount.google.com/permissions>.
    pub revoked: bool,
}

/// Managed state: the settings and the HTTP client that talks to Google.
#[derive(Debug, Clone)]
pub struct Google {
    pub config: Config,
    http: reqwest::Client,
}

impl Google {
    pub fn new(config: Config) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("the HTTP client builds");
        Self { config, http }
    }
}

/// Why [`access_token`] has no token to give. Messages never contain secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleError {
    /// These settings are missing.
    NotConfigured(Vec<&'static str>),
    /// No account is connected.
    NotConnected,
    /// The grant is gone or the stored tokens don't decrypt; the user must connect again.
    ReconnectRequired,
    /// Google's token endpoint failed: its `error` code or the transport error.
    Http(String),
    Database(String),
}

impl fmt::Display for GoogleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured(missing) => {
                write!(f, "Google is not configured: set {}", missing.join(", "))
            }
            Self::NotConnected => f.write_str("no Google account is connected"),
            Self::ReconnectRequired => f.write_str("the Google account must be connected again"),
            Self::Http(why) => write!(f, "Google's token endpoint failed: {why}"),
            Self::Database(why) => write!(f, "database error: {why}"),
        }
    }
}

impl std::error::Error for GoogleError {}

impl From<sqlx::Error> for GoogleError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err.to_string())
    }
}

/// A token endpoint response. Its `Debug` shows no token.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("scope", &self.scope)
            .finish()
    }
}

/// Google's error body; only `error` is read, so nothing else can reach a log.
#[derive(Deserialize)]
struct TokenErrorBody {
    error: String,
}

/// A failed token request.
#[derive(Debug)]
enum TokenError {
    /// Google answered non-2xx with this `error` code (`unknown` if it sent none).
    Google { status: u16, error: String },
    /// Network error, timeout or a 2xx body that didn't parse.
    Transport(String),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Google { status, error } => write!(f, "HTTP {status}, error {error:?}"),
            Self::Transport(why) => f.write_str(why),
        }
    }
}

/// POSTs `form` to the token endpoint.
async fn post_token(google: &Google, form: &[(&str, &str)]) -> Result<TokenResponse, TokenError> {
    let response = google
        .http
        .post(&google.config.token_url)
        .form(form)
        .send()
        .await
        .map_err(|err| TokenError::Transport(err.without_url().to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let error = response
            .json::<TokenErrorBody>()
            .await
            .map(|body| body.error)
            .unwrap_or_else(|_| "unknown".into());
        return Err(TokenError::Google {
            status: status.as_u16(),
            error,
        });
    }
    response
        .json::<TokenResponse>()
        .await
        .map_err(|_| TokenError::Transport("the token response did not parse".into()))
}

/// Revokes `token` at Google, best-effort. True when Google answered 2xx.
async fn revoke(google: &Google, token: &str) -> bool {
    let result = google
        .http
        .post(&google.config.revoke_url)
        .form(&[("token", token)])
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            rocket::warn!(
                "google: revoke answered HTTP {}",
                response.status().as_u16()
            );
            false
        }
        Err(err) => {
            rocket::warn!("google: revoke failed: {}", err.without_url());
            false
        }
    }
}

fn random_b64(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("the OS random number generator is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The PKCE S256 challenge for `verifier`.
fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn not_configured_message(missing: &[&str]) -> String {
    format!(
        "Google is not configured. Set {} in .env and restart the API (see docs/google-oauth.md).",
        missing.join(", ")
    )
}

/// Starts the Google sign-in: stores a fresh state and PKCE verifier and redirects to
/// Google's consent page. Open it in a browser.
#[utoipa::path(
    get,
    path = "/auth/google/connect",
    tag = "google",
    responses(
        (status = 303, description = "Redirects to Google's consent page", headers(
            ("Location" = String, description = "Google's authorization URL"),
        )),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site"),
        (status = 500, description = "The flow could not be stored"),
        (status = 503, description = "Google is not configured; the message names the missing settings", body = String, content_type = "text/plain"),
    )
)]
#[rocket::get("/auth/google/connect")]
pub async fn connect(
    _host: LocalHost,
    _site: NotCrossSite,
    google: &State<Google>,
    pool: &State<PgPool>,
) -> Result<Redirect, (Status, String)> {
    let creds = google
        .config
        .credentials()
        .map_err(|missing| (Status::ServiceUnavailable, not_configured_message(&missing)))?;

    let state = random_b64(32);
    let verifier = random_b64(64);
    let stored = async {
        sqlx::query(&format!(
            "DELETE FROM google_oauth_states \
             WHERE created_at < now() - interval '{STATE_TTL_MINUTES} minutes'"
        ))
        .execute(pool.inner())
        .await?;
        sqlx::query("INSERT INTO google_oauth_states (state, code_verifier) VALUES ($1, $2)")
            .bind(&state)
            .bind(&verifier)
            .execute(pool.inner())
            .await
    }
    .await;
    if let Err(err) = stored {
        rocket::warn!("google: cannot store the connect state: {err}");
        return Err((
            Status::InternalServerError,
            "The sign-in could not be started.".into(),
        ));
    }

    let url = authorization_url(&google.config, &creds, &state, &code_challenge(&verifier))
        .ok_or_else(|| {
            rocket::warn!("google: the authorization URL does not parse");
            (
                Status::InternalServerError,
                "The Google authorization URL is invalid.".to_string(),
            )
        })?;
    Ok(Redirect::to(url))
}

/// Google's consent URL; `None` only when `auth_url` is malformed.
fn authorization_url(
    config: &Config,
    creds: &Credentials<'_>,
    state: &str,
    challenge: &str,
) -> Option<String> {
    let scope = SCOPES.join(" ");
    reqwest::Url::parse_with_params(
        &config.auth_url,
        [
            ("response_type", "code"),
            ("client_id", creds.client_id),
            ("redirect_uri", config.redirect_uri.as_str()),
            ("scope", scope.as_str()),
            ("state", state),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ],
    )
    .ok()
    .map(String::from)
}

const PAGE_CONNECTED: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Waypoint</title></head><body><h1>Google connected</h1>\
<p>Google connected. You can close this tab.</p></body></html>";
const PAGE_INVALID_STATE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Waypoint</title></head><body><h1>Sign-in link expired</h1>\
<p>This sign-in link is invalid or expired. Start again from /auth/google/connect.</p>\
</body></html>";
const PAGE_CANCELLED: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Waypoint</title></head><body><h1>Google sign-in cancelled</h1>\
<p>The Google sign-in was cancelled or failed. Nothing was connected. Start again from \
/auth/google/connect.</p></body></html>";
const PAGE_SCOPES: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Waypoint</title></head><body><h1>Both permissions are needed</h1>\
<p>Waypoint needs both permissions: reading your Gmail and editing your calendar events. \
Nothing was connected. Start again from /auth/google/connect and leave both boxes \
ticked.</p></body></html>";
const PAGE_GOOGLE_FAILED: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Waypoint</title></head><body><h1>Google sign-in failed</h1>\
<p>Google did not complete the sign-in. Nothing was connected. Start again from \
/auth/google/connect.</p></body></html>";
const PAGE_NOT_CONFIGURED: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Waypoint</title></head><body><h1>Google is not configured</h1>\
<p>Set the Google settings in .env and restart the API (see docs/google-oauth.md).</p>\
</body></html>";
const PAGE_SERVER_ERROR: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Waypoint</title></head><body><h1>Something went wrong</h1>\
<p>Waypoint could not save the connection. Check the API log and start again from \
/auth/google/connect.</p></body></html>";

type Page = (Status, RawHtml<&'static str>);

fn page(status: Status, html: &'static str) -> Page {
    (status, RawHtml(html))
}

/// Where Google sends the browser back after consent. Exchanges the code, checks the
/// granted scopes and stores the sealed tokens. Answers a fixed HTML page.
#[utoipa::path(
    get,
    path = "/auth/google/callback",
    tag = "google",
    params(
        ("code" = Option<String>, Query, description = "The authorization code Google issued"),
        ("state" = Option<String>, Query, description = "The state `connect` created"),
        ("error" = Option<String>, Query, description = "Google's error, e.g. `access_denied`"),
    ),
    responses(
        (status = 200, description = "Google is connected", body = String, content_type = "text/html"),
        (status = 400, description = "The state is invalid or expired, the user cancelled, or a permission was not granted", body = String, content_type = "text/html"),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The connection could not be stored", body = String, content_type = "text/html"),
        (status = 502, description = "Google's token endpoint failed or returned no refresh token", body = String, content_type = "text/html"),
        (status = 503, description = "Google is not configured", body = String, content_type = "text/html"),
    )
)]
#[rocket::get("/auth/google/callback?<code>&<state>&<error>")]
pub async fn callback(
    _host: LocalHost,
    code: Option<&str>,
    state: Option<&str>,
    error: Option<&str>,
    google: &State<Google>,
    pool: &State<PgPool>,
) -> Page {
    let Ok(creds) = google.config.credentials() else {
        return page(Status::ServiceUnavailable, PAGE_NOT_CONFIGURED);
    };
    let Some(state) = state else {
        return page(Status::BadRequest, PAGE_INVALID_STATE);
    };

    // Consume the state whatever happens next, so it works once.
    let consumed: Result<Option<(String, bool)>, sqlx::Error> = sqlx::query_as(&format!(
        "DELETE FROM google_oauth_states WHERE state = $1 \
         RETURNING code_verifier, created_at > now() - interval '{STATE_TTL_MINUTES} minutes'"
    ))
    .bind(state)
    .fetch_optional(pool.inner())
    .await;
    let verifier = match consumed {
        Ok(Some((verifier, true))) => verifier,
        Ok(_) => return page(Status::BadRequest, PAGE_INVALID_STATE),
        Err(err) => {
            rocket::warn!("google: cannot read the connect state: {err}");
            return page(Status::InternalServerError, PAGE_SERVER_ERROR);
        }
    };

    if let Some(error) = error {
        // Google's error codes are short ASCII identifiers; cap it anyway.
        let error: String = error.chars().take(64).collect();
        rocket::info!("google: sign-in returned error {error:?}");
        return page(Status::BadRequest, PAGE_CANCELLED);
    }
    let Some(code) = code else {
        return page(Status::BadRequest, PAGE_CANCELLED);
    };

    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier.as_str()),
        ("redirect_uri", google.config.redirect_uri.as_str()),
        ("client_id", creds.client_id),
        ("client_secret", creds.client_secret),
    ];
    let tokens = match post_token(google, &form).await {
        Ok(tokens) => tokens,
        Err(err) => {
            rocket::warn!("google: code exchange failed: {err}");
            return page(Status::BadGateway, PAGE_GOOGLE_FAILED);
        }
    };

    let mut scopes: Vec<String> = tokens
        .scope
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    scopes.sort();
    scopes.dedup();
    if !SCOPES.iter().all(|s| scopes.iter().any(|g| g == s)) {
        rocket::warn!("google: the grant lacks a required scope (granted {scopes:?})");
        let token = tokens
            .refresh_token
            .as_deref()
            .unwrap_or(&tokens.access_token);
        revoke(google, token).await;
        return page(Status::BadRequest, PAGE_SCOPES);
    }
    let Some(refresh_token) = tokens.refresh_token.as_deref() else {
        rocket::warn!("google: the code exchange returned no refresh token");
        return page(Status::BadGateway, PAGE_GOOGLE_FAILED);
    };

    let key = creds.token_key;
    let stored = sqlx::query(
        "INSERT INTO google_accounts \
             (id, refresh_token, access_token, access_token_expires_at, scopes, status, \
              connected_at, updated_at) \
         VALUES (true, $1, $2, now() + $3 * interval '1 second', $4, 'connected', now(), now()) \
         ON CONFLICT (id) DO UPDATE SET \
             refresh_token = EXCLUDED.refresh_token, access_token = EXCLUDED.access_token, \
             access_token_expires_at = EXCLUDED.access_token_expires_at, \
             scopes = EXCLUDED.scopes, status = 'connected', connected_at = now(), \
             updated_at = now()",
    )
    .bind(crypto::seal(
        key,
        REFRESH_TOKEN_AAD,
        refresh_token.as_bytes(),
    ))
    .bind(crypto::seal(
        key,
        ACCESS_TOKEN_AAD,
        tokens.access_token.as_bytes(),
    ))
    .bind(tokens.expires_in.unwrap_or(DEFAULT_EXPIRES_IN))
    .bind(&scopes)
    .execute(pool.inner())
    .await;
    match stored {
        Ok(_) => page(Status::Ok, PAGE_CONNECTED),
        Err(err) => {
            rocket::warn!("google: cannot store the account: {err}");
            page(Status::InternalServerError, PAGE_SERVER_ERROR)
        }
    }
}

/// Reports whether Google is configured and connected. Never decrypts or calls Google.
#[utoipa::path(
    get,
    path = "/auth/google",
    tag = "google",
    responses(
        (status = 200, description = "The Google connection's status", body = GoogleStatus),
        (status = 403, description = "The `Host` header is not a local address"),
        (status = 500, description = "The status could not be read"),
    )
)]
#[rocket::get("/auth/google")]
pub async fn status(
    _host: LocalHost,
    google: &State<Google>,
    pool: &State<PgPool>,
) -> Result<Json<GoogleStatus>, Status> {
    let row: Option<(String, Vec<String>, DateTime<Utc>)> =
        sqlx::query_as("SELECT status, scopes, connected_at FROM google_accounts WHERE id")
            .fetch_optional(pool.inner())
            .await
            .map_err(|err| {
                rocket::warn!("google: cannot read the account: {err}");
                Status::InternalServerError
            })?;
    let configured = google.config.is_configured();
    let status = match row {
        None => GoogleStatus {
            configured,
            state: ConnectionState::Disconnected,
            scopes: Vec::new(),
            connected_at: None,
        },
        Some((status, mut scopes, connected_at)) => {
            scopes.sort();
            GoogleStatus {
                configured,
                state: if status == "connected" {
                    ConnectionState::Connected
                } else {
                    ConnectionState::ReconnectRequired
                },
                scopes,
                connected_at: Some(connected_at),
            }
        }
    };
    Ok(Json(status))
}

/// Disconnects Google: revokes the grant (best-effort) and deletes the stored tokens.
#[utoipa::path(
    delete,
    path = "/auth/google",
    tag = "google",
    responses(
        (status = 200, description = "The tokens are deleted; `revoked` says whether Google confirmed the revocation", body = GoogleDisconnect),
        (status = 403, description = "The `Host` header is not a local address, or the request came from another site or as an HTML form"),
        (status = 404, description = "No Google account is connected"),
        (status = 500, description = "The account could not be deleted"),
    )
)]
#[rocket::delete("/auth/google")]
pub async fn disconnect(
    _host: LocalHost,
    _site: NotCrossSite,
    google: &State<Google>,
    pool: &State<PgPool>,
) -> Result<Json<GoogleDisconnect>, Status> {
    let deleted: Option<(Vec<u8>,)> =
        sqlx::query_as("DELETE FROM google_accounts WHERE id RETURNING refresh_token")
            .fetch_optional(pool.inner())
            .await
            .map_err(|err| {
                rocket::warn!("google: cannot delete the account: {err}");
                Status::InternalServerError
            })?;
    let Some((sealed,)) = deleted else {
        return Err(Status::NotFound);
    };
    let token = google
        .config
        .token_key
        .as_ref()
        .and_then(|key| crypto::open(key, REFRESH_TOKEN_AAD, &sealed).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let revoked = match token {
        Some(token) => revoke(google, &token).await,
        None => {
            rocket::warn!("google: the stored refresh token does not decrypt; not revoked");
            false
        }
    };
    if !revoked {
        rocket::warn!(
            "google: disconnected without revoking; remove the grant at \
             https://myaccount.google.com/permissions"
        );
    }
    Ok(Json(GoogleDisconnect { revoked }))
}

fn open_token(key: &TokenKey, aad: &[u8], sealed: &[u8]) -> Option<String> {
    crypto::open(key, aad, sealed)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

async fn mark_reconnect_required<'e, E>(executor: E) -> Result<(), sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query(
        "UPDATE google_accounts SET status = 'reconnect_required', updated_at = now() WHERE id",
    )
    .execute(executor)
    .await
    .map(|_| ())
}

/// A Google access token valid for at least another minute. Returns the stored one when
/// it is, and otherwise refreshes it while holding the account row's lock, so concurrent
/// callers refresh once. A revoked grant (`invalid_grant`) or a stored token that no
/// longer decrypts marks the account `reconnect_required`.
pub async fn access_token(pool: &PgPool, google: &Google) -> Result<String, GoogleError> {
    let creds = google
        .config
        .credentials()
        .map_err(GoogleError::NotConfigured)?;
    let key = creds.token_key;
    let fresh_sql = format!(
        "SELECT status, access_token, \
         access_token_expires_at > now() + interval '{REFRESH_MARGIN_SECS} seconds' \
         FROM google_accounts WHERE id"
    );

    // Fast path: no lock while the stored token is fresh.
    let row: Option<(String, Vec<u8>, bool)> =
        sqlx::query_as(&fresh_sql).fetch_optional(pool).await?;
    match row {
        None => return Err(GoogleError::NotConnected),
        Some((status, _, _)) if status != "connected" => {
            return Err(GoogleError::ReconnectRequired);
        }
        Some((_, sealed, true)) => {
            return match open_token(key, ACCESS_TOKEN_AAD, &sealed) {
                Some(token) => Ok(token),
                None => {
                    rocket::warn!("google: the stored access token does not decrypt");
                    mark_reconnect_required(pool).await?;
                    Err(GoogleError::ReconnectRequired)
                }
            };
        }
        Some(_) => {}
    }

    let mut tx = pool.begin().await?;
    let row: Option<(String, Vec<u8>, bool, Vec<u8>)> = sqlx::query_as(&format!(
        "SELECT status, access_token, \
         access_token_expires_at > now() + interval '{REFRESH_MARGIN_SECS} seconds', \
         refresh_token FROM google_accounts WHERE id FOR UPDATE"
    ))
    .fetch_optional(&mut *tx)
    .await?;
    let (sealed_access, fresh, sealed_refresh) = match row {
        None => return Err(GoogleError::NotConnected),
        Some((status, _, _, _)) if status != "connected" => {
            return Err(GoogleError::ReconnectRequired);
        }
        Some((_, access, fresh, refresh)) => (access, fresh, refresh),
    };

    // Another caller may have refreshed while this one waited for the lock.
    let sealed = if fresh {
        (ACCESS_TOKEN_AAD, sealed_access)
    } else {
        (REFRESH_TOKEN_AAD, sealed_refresh)
    };
    let Some(token) = open_token(key, sealed.0, &sealed.1) else {
        rocket::warn!("google: a stored token does not decrypt");
        mark_reconnect_required(&mut *tx).await?;
        tx.commit().await?;
        return Err(GoogleError::ReconnectRequired);
    };
    if fresh {
        return Ok(token);
    }

    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", token.as_str()),
        ("client_id", creds.client_id),
        ("client_secret", creds.client_secret),
    ];
    let tokens = match post_token(google, &form).await {
        Ok(tokens) => tokens,
        Err(TokenError::Google { error, .. }) if error == "invalid_grant" => {
            rocket::warn!("google: the grant was revoked or expired; reconnect required");
            mark_reconnect_required(&mut *tx).await?;
            tx.commit().await?;
            return Err(GoogleError::ReconnectRequired);
        }
        Err(err) => {
            rocket::warn!("google: token refresh failed: {err}");
            return Err(GoogleError::Http(err.to_string()));
        }
    };

    let rotated = tokens
        .refresh_token
        .as_deref()
        .map(|refresh| crypto::seal(key, REFRESH_TOKEN_AAD, refresh.as_bytes()));
    sqlx::query(
        "UPDATE google_accounts SET access_token = $1, \
         access_token_expires_at = now() + $2 * interval '1 second', \
         refresh_token = COALESCE($3, refresh_token), updated_at = now() WHERE id",
    )
    .bind(crypto::seal(
        key,
        ACCESS_TOKEN_AAD,
        tokens.access_token.as_bytes(),
    ))
    .bind(tokens.expires_in.unwrap_or(DEFAULT_EXPIRES_IN))
    .bind(rotated)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(tokens.access_token)
}
