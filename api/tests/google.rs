//! Google OAuth against a mocked Google (wiremock): connect, callback, status, disconnect
//! and the access-token refresh.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::crypto::{self, TokenKey};
use waypoint_api::google::{self, Google, GoogleError};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const KEY: [u8; 32] = [7; 32];
const CLIENT_ID: &str = "test-client.apps.googleusercontent.com";

/// Fake credentials, built at runtime so they never look like committed secrets.
fn fake(kind: &str) -> String {
    format!("fake-{kind}-{}", "for-tests")
}

/// The PKCE S256 challenge (RFC 7636): base64url without padding of SHA-256(verifier).
fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn config(server: &MockServer) -> google::Config {
    google::Config {
        client_id: Some(CLIENT_ID.into()),
        client_secret: Some(fake("client-secret")),
        token_key: Some(TokenKey::new(KEY)),
        auth_url: format!("{}/auth", server.uri()),
        token_url: format!("{}/token", server.uri()),
        revoke_url: format!("{}/revoke", server.uri()),
        redirect_uri: google::config::redirect_uri(8000),
        // Startup catch-up may sync against the mock, never the real Gmail.
        gmail: waypoint_api::gmail::Config {
            base_url: server.uri(),
            ..Default::default()
        },
    }
}

async fn app(pool: PgPool, config: google::Config) -> Client {
    let rocket = waypoint_api::rocket(
        rocket::Config::figment(),
        pool,
        ClaudeRunner::new("claude", Duration::from_secs(1)),
        waypoint_api::jobs::Config::default(),
        config,
        waypoint_api::capture::Config::default(),
    );
    Client::tracked(rocket).await.expect("valid rocket")
}

fn host() -> Header<'static> {
    Header::new("Host", "127.0.0.1")
}

/// The form fields of a recorded request.
fn form(request: &Request) -> HashMap<String, String> {
    reqwest::Url::parse(&format!(
        "http://x/?{}",
        String::from_utf8_lossy(&request.body)
    ))
    .expect("form body")
    .query_pairs()
    .into_owned()
    .collect()
}

async fn requests_to(server: &MockServer, to: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("recording is on")
        .into_iter()
        .filter(|r| r.url.path() == to)
        .collect()
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

fn both_scopes() -> String {
    format!("{} {}", google::GMAIL_READONLY, google::CALENDAR_EVENTS)
}

fn token_body(access: &str, refresh: Option<&str>, scope: &str) -> Value {
    let mut body = json!({
        "access_token": access,
        "expires_in": 3599,
        "scope": scope,
        "token_type": "Bearer",
    });
    if let Some(refresh) = refresh {
        body["refresh_token"] = json!(refresh);
    }
    body
}

/// Starts a flow and returns the `Location` query parameters.
async fn connect(client: &Client) -> HashMap<String, String> {
    let response = client
        .get("/auth/google/connect")
        .header(host())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::SeeOther);
    let location = response.headers().get_one("Location").expect("Location");
    reqwest::Url::parse(location)
        .expect("absolute URL")
        .query_pairs()
        .into_owned()
        .collect()
}

async fn callback<'c>(client: &'c Client, query: &str) -> LocalResponse<'c> {
    client
        .get(format!("/auth/google/callback?{query}"))
        .header(host())
        .dispatch()
        .await
}

async fn status(client: &Client) -> Value {
    let response = client.get("/auth/google").header(host()).dispatch().await;
    assert_eq!(response.status(), Status::Ok);
    response.into_json().await.expect("json")
}

/// Stores a connected account whose access token expires in `expires_in` seconds.
async fn seed(pool: &PgPool, access: &str, refresh: &str, expires_in: i64) {
    let key = TokenKey::new(KEY);
    sqlx::query(
        "INSERT INTO google_accounts \
             (refresh_token, access_token, access_token_expires_at, scopes, status) \
         VALUES ($1, $2, now() + $3 * interval '1 second', $4, 'connected')",
    )
    .bind(crypto::seal(
        &key,
        google::REFRESH_TOKEN_AAD,
        refresh.as_bytes(),
    ))
    .bind(crypto::seal(
        &key,
        google::ACCESS_TOKEN_AAD,
        access.as_bytes(),
    ))
    .bind(expires_in)
    .bind(google::SCOPES.map(String::from).to_vec())
    .execute(pool)
    .await
    .expect("seed account");
}

/// The decrypted stored tokens and the seconds until the access token expires.
async fn stored(pool: &PgPool) -> (String, String, f64) {
    let (refresh, access, secs): (Vec<u8>, Vec<u8>, f64) = sqlx::query_as(
        "SELECT refresh_token, access_token, \
         EXTRACT(EPOCH FROM access_token_expires_at - now())::float8 FROM google_accounts",
    )
    .fetch_one(pool)
    .await
    .expect("account row");
    let key = TokenKey::new(KEY);
    let open = |aad, blob: &[u8]| {
        String::from_utf8(crypto::open(&key, aad, blob).expect("decrypts")).expect("utf-8")
    };
    (
        open(google::REFRESH_TOKEN_AAD, &refresh),
        open(google::ACCESS_TOKEN_AAD, &access),
        secs,
    )
}

#[sqlx::test]
async fn connect_redirects_with_pkce_and_narrow_scopes(pool: PgPool) {
    let server = MockServer::start().await;
    let client = app(pool.clone(), config(&server)).await;

    let response = client
        .get("/auth/google/connect")
        .header(host())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::SeeOther);
    let location = response.headers().get_one("Location").expect("Location");
    assert!(
        location.starts_with(&format!("{}/auth?", server.uri())),
        "{location}"
    );
    let params = connect(&client).await;

    let expected = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", "http://127.0.0.1:8000/auth/google/callback"),
        (
            "scope",
            "https://www.googleapis.com/auth/gmail.readonly \
             https://www.googleapis.com/auth/calendar.events",
        ),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    for (name, value) in expected {
        assert_eq!(params.get(name).map(String::as_str), Some(value), "{name}");
    }
    assert!(!params.contains_key("include_granted_scopes"));
    assert_eq!(params.len(), 9, "{params:?}");

    let state = &params["state"];
    assert_eq!(state.len(), 43, "32 bytes, base64url");
    let verifier: String =
        sqlx::query_scalar("SELECT code_verifier FROM google_oauth_states WHERE state = $1")
            .bind(state)
            .fetch_one(&pool)
            .await
            .expect("a state row for the returned state");
    assert_eq!(verifier.len(), 86);
    assert_eq!(params["code_challenge"], s256(&verifier));
    assert_eq!(count(&pool, "google_oauth_states").await, 2);
}

#[sqlx::test]
async fn connect_not_configured(pool: PgPool) {
    let client = app(pool.clone(), google::Config::default()).await;
    let response = client
        .get("/auth/google/connect")
        .header(host())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::ServiceUnavailable);
    assert_eq!(response.content_type(), Some(ContentType::Plain));
    let body = response.into_string().await.expect("body");
    for name in [
        "WAYPOINT_GOOGLE_CLIENT_ID",
        "WAYPOINT_GOOGLE_CLIENT_SECRET",
        "WAYPOINT_TOKEN_KEY",
    ] {
        assert!(body.contains(name), "{body}");
    }

    let server = MockServer::start().await;
    let partial = google::Config {
        token_key: None,
        ..config(&server)
    };
    let client = app(pool.clone(), partial).await;
    let response = client
        .get("/auth/google/connect")
        .header(host())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::ServiceUnavailable);
    let body = response.into_string().await.expect("body");
    assert!(body.contains("WAYPOINT_TOKEN_KEY"), "{body}");
    assert!(!body.contains("WAYPOINT_GOOGLE_CLIENT_ID"), "{body}");
    assert_eq!(count(&pool, "google_oauth_states").await, 0);
}

#[sqlx::test]
async fn connect_rejects_cross_site(pool: PgPool) {
    let server = MockServer::start().await;
    let client = app(pool.clone(), config(&server)).await;

    let response = client
        .get("/auth/google/connect")
        .header(host())
        .header(Header::new("Sec-Fetch-Site", "cross-site"))
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Forbidden);
    assert_eq!(count(&pool, "google_oauth_states").await, 0);

    for site in ["same-origin", "same-site", "none"] {
        let response = client
            .get("/auth/google/connect")
            .header(host())
            .header(Header::new("Sec-Fetch-Site", site))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::SeeOther, "{site}");
    }
}

#[sqlx::test]
async fn callback_exchanges_code_and_connects(pool: PgPool) {
    let server = MockServer::start().await;
    let access = fake("access-token");
    let refresh = fake("refresh-token");
    // Google lists granted scopes in its own order.
    let granted = format!("{} {}", google::CALENDAR_EVENTS, google::GMAIL_READONLY);
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(
            &access,
            Some(&refresh),
            &granted,
        )))
        .expect(1)
        .mount(&server)
        .await;
    let client = app(pool.clone(), config(&server)).await;

    let params = connect(&client).await;
    let state = &params["state"];
    let response = callback(
        &client,
        &format!("state={state}&code=4%2F0AUTHCODE&scope=ignored&authuser=0"),
    )
    .await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.content_type(), Some(ContentType::HTML));
    let body = response.into_string().await.expect("body");
    assert!(body.contains("Google connected"), "{body}");
    assert_eq!(count(&pool, "google_oauth_states").await, 0);

    let requests = requests_to(&server, "/token").await;
    assert_eq!(requests.len(), 1);
    let sent = form(&requests[0]);
    assert_eq!(sent["grant_type"], "authorization_code");
    assert_eq!(sent["code"], "4/0AUTHCODE");
    assert_eq!(
        sent["redirect_uri"],
        "http://127.0.0.1:8000/auth/google/callback"
    );
    assert_eq!(sent["client_id"], CLIENT_ID);
    assert_eq!(sent["client_secret"], fake("client-secret"));
    assert_eq!(s256(&sent["code_verifier"]), params["code_challenge"]);
    assert_eq!(sent.len(), 6, "{:?}", sent.keys());

    let status = status(&client).await;
    assert_eq!(status["configured"], true);
    assert_eq!(status["state"], "connected");
    assert_eq!(
        status["scopes"],
        json!([google::CALENDAR_EVENTS, google::GMAIL_READONLY])
    );
    assert!(status["connected_at"].is_string());

    // AC6: the columns hold ciphertext that opens with the configured key.
    let (raw_refresh, raw_access): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT refresh_token, access_token FROM google_accounts")
            .fetch_one(&pool)
            .await
            .expect("account row");
    for raw in [&raw_refresh, &raw_access] {
        for plain in [&access, &refresh] {
            assert!(
                !raw.windows(plain.len()).any(|w| w == plain.as_bytes()),
                "a column holds a plaintext token"
            );
        }
    }
    let (stored_refresh, stored_access, secs) = stored(&pool).await;
    assert_eq!(stored_refresh, refresh);
    assert_eq!(stored_access, access);
    assert!((3500.0..=3600.0).contains(&secs), "{secs}");
}

#[sqlx::test]
async fn callback_rejects_bad_state(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = app(pool.clone(), config(&server)).await;

    let invalid = "This sign-in link is invalid or expired";
    let mut queries = vec!["code=abc".to_string(), "state=unknown&code=abc".to_string()];

    // Reused: the first use (a cancelled sign-in) consumes the state.
    let state = connect(&client).await["state"].clone();
    let first = callback(&client, &format!("state={state}&error=access_denied")).await;
    assert_eq!(first.status(), Status::BadRequest);
    queries.push(format!("state={state}&code=abc"));

    // Expired: older than 10 minutes.
    let old = connect(&client).await["state"].clone();
    sqlx::query(
        "UPDATE google_oauth_states SET created_at = now() - interval '11 minutes' \
         WHERE state = $1",
    )
    .bind(&old)
    .execute(&pool)
    .await
    .expect("backdate");
    queries.push(format!("state={old}&code=abc"));

    for query in &queries {
        let response = callback(&client, query).await;
        assert_eq!(response.status(), Status::BadRequest, "{query}");
        let body = response.into_string().await.expect("body");
        assert!(body.contains(invalid), "{query}: {body}");
        assert!(!body.contains("abc") && !body.contains("unknown"), "{body}");
    }
    assert_eq!(count(&pool, "google_oauth_states").await, 0);
    assert_eq!(count(&pool, "google_accounts").await, 0);
}

/// One failed callback: Google's `error` (or `None` to send a code), the token
/// endpoint's answer (`None`: it must not be called), the expected status and how many
/// revocations it makes.
struct Case {
    name: &'static str,
    error: Option<&'static str>,
    token: Option<ResponseTemplate>,
    status: Status,
    revokes: u64,
}

#[sqlx::test]
async fn callback_failures_store_nothing(pool: PgPool) {
    let refresh = fake("refresh-token");
    let gmail_only = google::GMAIL_READONLY.to_string();
    let cases = [
        Case {
            name: "access denied",
            error: Some("access_denied"),
            token: None,
            status: Status::BadRequest,
            revokes: 0,
        },
        Case {
            name: "token endpoint 4xx",
            error: None,
            token: Some(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant",
                "error_description": "Bad Request",
            }))),
            status: Status::BadGateway,
            revokes: 0,
        },
        Case {
            name: "token endpoint 5xx",
            error: None,
            token: Some(ResponseTemplate::new(503).set_body_string("unavailable")),
            status: Status::BadGateway,
            revokes: 0,
        },
        Case {
            name: "no refresh token",
            error: None,
            token: Some(ResponseTemplate::new(200).set_body_json(token_body(
                "access",
                None,
                &both_scopes(),
            ))),
            status: Status::BadGateway,
            revokes: 0,
        },
        Case {
            name: "scope missing",
            error: None,
            token: Some(ResponseTemplate::new(200).set_body_json(token_body(
                "access",
                Some(&refresh),
                &gmail_only,
            ))),
            status: Status::BadRequest,
            revokes: 1,
        },
    ];

    for Case {
        name: case,
        error,
        token,
        status: expected,
        revokes,
    } in cases
    {
        let server = MockServer::start().await;
        let token_calls = u64::from(token.is_some());
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(token.unwrap_or_else(|| ResponseTemplate::new(500)))
            .expect(token_calls)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .respond_with(ResponseTemplate::new(200))
            .expect(revokes)
            .mount(&server)
            .await;
        let client = app(pool.clone(), config(&server)).await;

        let state = connect(&client).await["state"].clone();
        let query = match error {
            Some(error) => format!("state={state}&error={error}"),
            None => format!("state={state}&code=abc"),
        };
        let response = callback(&client, &query).await;
        assert_eq!(response.status(), expected, "{case}");
        assert_eq!(response.content_type(), Some(ContentType::HTML), "{case}");
        assert_eq!(count(&pool, "google_accounts").await, 0, "{case}");
        assert_eq!(count(&pool, "google_oauth_states").await, 0, "{case}");
        if revokes > 0 {
            let revoked = requests_to(&server, "/revoke").await;
            assert_eq!(form(&revoked[0])["token"], refresh, "{case}");
        }
        server.verify().await;
    }
}

#[sqlx::test]
async fn status_reports_disconnected(pool: PgPool) {
    let server = MockServer::start().await;
    let client = app(pool.clone(), config(&server)).await;
    assert_eq!(
        status(&client).await,
        json!({"configured": true, "state": "disconnected", "scopes": [], "connected_at": null})
    );

    let client = app(pool, google::Config::default()).await;
    assert_eq!(
        status(&client).await,
        json!({"configured": false, "state": "disconnected", "scopes": [], "connected_at": null})
    );
}

#[sqlx::test]
async fn access_token_uses_cached_token(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let google = Google::new(config(&server));

    assert_eq!(
        google::access_token(&pool, &google).await,
        Err(GoogleError::NotConnected)
    );
    seed(&pool, "cached-access", "refresh", 3600).await;
    assert_eq!(
        google::access_token(&pool, &google).await.as_deref(),
        Ok("cached-access")
    );
}

#[sqlx::test]
async fn access_token_refreshes_and_rotates(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(
            "new-access",
            Some("rotated-refresh"),
            &both_scopes(),
        )))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(
            "newer-access",
            None,
            &both_scopes(),
        )))
        .mount(&server)
        .await;
    let google = Google::new(config(&server));
    seed(&pool, "old-access", "old-refresh", 30).await;

    assert_eq!(
        google::access_token(&pool, &google).await.as_deref(),
        Ok("new-access")
    );
    let requests = requests_to(&server, "/token").await;
    let sent = form(&requests[0]);
    assert_eq!(sent["grant_type"], "refresh_token");
    assert_eq!(sent["refresh_token"], "old-refresh");
    assert_eq!(sent["client_id"], CLIENT_ID);
    assert_eq!(sent["client_secret"], fake("client-secret"));
    assert_eq!(sent.len(), 4);
    let (refresh, access, secs) = stored(&pool).await;
    assert_eq!(
        (refresh.as_str(), access.as_str()),
        ("rotated-refresh", "new-access")
    );
    assert!(secs > 3500.0, "{secs}");

    // Without a new refresh token in the response, the stored one is kept.
    sqlx::query("UPDATE google_accounts SET access_token_expires_at = now()")
        .execute(&pool)
        .await
        .expect("expire");
    assert_eq!(
        google::access_token(&pool, &google).await.as_deref(),
        Ok("newer-access")
    );
    let (refresh, access, _) = stored(&pool).await;
    assert_eq!(
        (refresh.as_str(), access.as_str()),
        ("rotated-refresh", "newer-access")
    );
    assert_eq!(
        form(&requests_to(&server, "/token").await[1])["refresh_token"],
        "rotated-refresh"
    );
}

#[sqlx::test]
async fn access_token_refreshes_once_when_concurrent(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(token_body("fresh-access", None, &both_scopes()))
                .set_delay(Duration::from_millis(300)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let google = Google::new(config(&server));
    seed(&pool, "stale-access", "refresh", 10).await;

    let (a, b) = tokio::join!(
        google::access_token(&pool, &google),
        google::access_token(&pool, &google)
    );
    assert_eq!(a.as_deref(), Ok("fresh-access"));
    assert_eq!(b.as_deref(), Ok("fresh-access"));
    server.verify().await;
}

#[sqlx::test]
async fn access_token_invalid_grant_requires_reconnect(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant",
            "error_description": "Token has been expired or revoked.",
        })))
        .expect(1)
        .mount(&server)
        .await;
    let config = config(&server);
    let google = Google::new(config.clone());
    seed(&pool, "stale-access", "revoked-refresh", 10).await;

    assert_eq!(
        google::access_token(&pool, &google).await,
        Err(GoogleError::ReconnectRequired)
    );
    // Later calls fail fast without calling Google again.
    assert_eq!(
        google::access_token(&pool, &google).await,
        Err(GoogleError::ReconnectRequired)
    );
    let client = app(pool.clone(), config).await;
    assert_eq!(status(&client).await["state"], "reconnect_required");
}

#[sqlx::test]
async fn disconnect_revokes_and_deletes(pool: PgPool) {
    let server = MockServer::start().await;
    let client = app(pool.clone(), config(&server)).await;
    let disconnect = || client.delete("/auth/google").header(host()).dispatch();

    // Google confirms the revocation.
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    seed(&pool, "access", "refresh-to-revoke", 3600).await;
    let response = disconnect().await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.into_json::<Value>().await,
        Some(json!({"revoked": true}))
    );
    let revoked = requests_to(&server, "/revoke").await;
    assert_eq!(form(&revoked[0])["token"], "refresh-to-revoke");
    assert_eq!(count(&pool, "google_accounts").await, 0);
    server.verify().await;
    server.reset().await;

    // Google fails: the row is deleted anyway.
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    seed(&pool, "access", "refresh", 3600).await;
    let response = disconnect().await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.into_json::<Value>().await,
        Some(json!({"revoked": false}))
    );
    assert_eq!(count(&pool, "google_accounts").await, 0);

    // Nothing to disconnect.
    assert_eq!(disconnect().await.status(), Status::NotFound);
}

#[sqlx::test]
async fn disconnect_rejects_cross_site_and_form_override(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = app(pool.clone(), config(&server)).await;
    seed(&pool, "access", "refresh", 3600).await;

    // What any web page can auto-submit: a plain HTML form that Rocket turns into DELETE.
    for site in [Some("cross-site"), None] {
        let mut request = client
            .post("/auth/google")
            .header(host())
            .header(ContentType::Form)
            .body("_method=delete");
        if let Some(site) = site {
            request = request.header(Header::new("Sec-Fetch-Site", site));
        }
        assert_eq!(
            request.dispatch().await.status(),
            Status::Forbidden,
            "{site:?}"
        );
    }
    let response = client
        .delete("/auth/google")
        .header(host())
        .header(Header::new("Sec-Fetch-Site", "cross-site"))
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Forbidden);
    assert_eq!(count(&pool, "google_accounts").await, 1);
}

#[sqlx::test]
async fn undecryptable_tokens_require_reconnect(pool: PgPool) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    // The tokens were sealed with KEY; the API now runs with another key.
    let rotated = google::Config {
        token_key: Some(TokenKey::new([8; 32])),
        ..config(&server)
    };
    let google = Google::new(rotated.clone());
    let client = app(pool.clone(), rotated).await;

    for expires_in in [3600, 10] {
        sqlx::query("DELETE FROM google_accounts")
            .execute(&pool)
            .await
            .expect("clear account");
        seed(&pool, "access", "refresh", expires_in).await;
        assert_eq!(
            google::access_token(&pool, &google).await,
            Err(GoogleError::ReconnectRequired),
            "expires in {expires_in}s"
        );
        assert_eq!(status(&client).await["state"], "reconnect_required");
    }

    // Disconnect can't revoke what it can't decrypt, but still deletes the row.
    let response = client
        .delete("/auth/google")
        .header(host())
        .dispatch()
        .await;
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.into_json::<Value>().await,
        Some(json!({"revoked": false}))
    );
    assert_eq!(count(&pool, "google_accounts").await, 0);
}
