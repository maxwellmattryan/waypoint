//! A read-only Gmail REST client: `users.getProfile`, `messages.list`,
//! `messages.get?format=raw` and `history.list`, each a `GET` with a `Bearer` token.
//! Responses are trimmed to the fields the sync uses. Errors never carry the token, a URL,
//! a message body or a subject.

use std::fmt;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use reqwest::{Response, StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Largest JSON body read for a profile, list or history page.
const JSON_LIMIT: usize = 16 * 1024 * 1024;
/// Largest error body read when looking for the error reason.
const ERROR_LIMIT: usize = 64 * 1024;
/// Slack over the base64 size of a capped message for the JSON around `raw`.
const RAW_SLACK: usize = 64 * 1024;

/// Why a Gmail call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmailError {
    /// HTTP 401: the access token was refused.
    Unauthorized,
    /// HTTP 404 on `history.list`: the start `historyId` is too old.
    HistoryExpired,
    /// HTTP 404 on `messages.get`: the message was deleted.
    NotFound,
    /// HTTP 429, or 403 with reason `rateLimitExceeded` / `userRateLimitExceeded`.
    RateLimited { retry_after: Option<Duration> },
    /// The message is over the size cap; it was not read in full.
    Oversized,
    /// Any other non-2xx answer.
    Http { status: u16 },
    /// Network error or timeout.
    Transport(String),
    /// A 2xx body that didn't parse.
    Decode(String),
}

impl fmt::Display for GmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("Gmail refused the access token (HTTP 401)"),
            Self::HistoryExpired => f.write_str("Gmail's history has expired (HTTP 404)"),
            Self::NotFound => f.write_str("the Gmail message no longer exists (HTTP 404)"),
            Self::RateLimited {
                retry_after: Some(after),
            } => write!(
                f,
                "Gmail's rate limit was hit; retry after {}s",
                after.as_secs()
            ),
            Self::RateLimited { retry_after: None } => f.write_str("Gmail's rate limit was hit"),
            Self::Oversized => f.write_str("the Gmail message is over the size cap"),
            Self::Http { status } => write!(f, "Gmail answered HTTP {status}"),
            Self::Transport(why) => write!(f, "cannot reach Gmail: {why}"),
            Self::Decode(why) => write!(f, "Gmail's response did not parse: {why}"),
        }
    }
}

impl std::error::Error for GmailError {}

/// `users.getProfile`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub email_address: String,
    pub history_id: String,
}

/// One id from `messages.list`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRef {
    pub id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// One `messages.list` page.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageList {
    #[serde(default)]
    pub messages: Vec<MessageRef>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// A message from `messages.get?format=raw`, with `raw` decoded. Its `Debug` omits the
/// bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct RawMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub label_ids: Vec<String>,
    /// The RFC 822 bytes.
    pub raw: Vec<u8>,
}

impl fmt::Debug for RawMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawMessage")
            .field("id", &self.id)
            .field("thread_id", &self.thread_id)
            .field("label_ids", &self.label_ids)
            .field("raw", &format_args!("<{} bytes>", self.raw.len()))
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawWire {
    id: String,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    label_ids: Vec<String>,
    raw: String,
}

/// A message in a history record.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMessage {
    pub id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MessageAdded {
    pub message: HistoryMessage,
}

/// One `history.list` record; only `messagesAdded` is read.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    #[serde(default)]
    pub messages_added: Vec<MessageAdded>,
}

/// One `history.list` page.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    #[serde(default)]
    pub history: Vec<HistoryRecord>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    pub history_id: String,
}

/// Gmail's error body; only the first reason is read.
#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    errors: Vec<ErrorItem>,
}

#[derive(Deserialize)]
struct ErrorItem {
    #[serde(default)]
    reason: Option<String>,
}

/// The Gmail API for one access token. No `Debug`, so the token can't reach a log.
pub struct GmailClient<'a> {
    http: &'a reqwest::Client,
    base: &'a str,
    token: &'a str,
}

/// What a 404 means for the endpoint being called.
#[derive(Clone, Copy)]
enum On404 {
    Http,
    HistoryExpired,
    NotFound,
}

impl<'a> GmailClient<'a> {
    /// `base` is the API origin, e.g. `https://gmail.googleapis.com`.
    pub fn new(http: &'a reqwest::Client, base: &'a str, token: &'a str) -> Self {
        Self { http, base, token }
    }

    pub async fn profile(&self) -> Result<Profile, GmailError> {
        let response = self.get(&["profile"], &[], On404::Http).await?;
        json(response, "the profile").await
    }

    /// One page of message ids matching `q`, newest first.
    pub async fn list_messages(
        &self,
        q: &str,
        page_token: Option<&str>,
        max: u32,
    ) -> Result<MessageList, GmailError> {
        let mut query = vec![("q", q.to_string()), ("maxResults", max.to_string())];
        if let Some(token) = page_token {
            query.push(("pageToken", token.to_string()));
        }
        let response = self.get(&["messages"], &query, On404::Http).await?;
        json(response, "the message list").await
    }

    /// One message's raw bytes. The body is read in chunks and abandoned past
    /// `max_bytes * 4 / 3 + 64 KiB`, and a decoded message over `max_bytes` is refused;
    /// both are [`GmailError::Oversized`].
    pub async fn get_raw(&self, id: &str, max_bytes: usize) -> Result<RawMessage, GmailError> {
        let query = [("format", "raw".to_string())];
        let response = self.get(&["messages", id], &query, On404::NotFound).await?;
        let limit = (max_bytes.saturating_mul(4) / 3).saturating_add(RAW_SLACK);
        let body = read_capped(response, limit)
            .await?
            .ok_or(GmailError::Oversized)?;
        let wire: RawWire = parse(&body, "the message")?;
        let raw = URL_SAFE
            .decode(wire.raw.as_bytes())
            .or_else(|_| URL_SAFE_NO_PAD.decode(wire.raw.as_bytes()))
            .map_err(|_| GmailError::Decode("the message's raw field is not base64url".into()))?;
        if raw.len() > max_bytes {
            return Err(GmailError::Oversized);
        }
        Ok(RawMessage {
            id: wire.id,
            thread_id: wire.thread_id,
            label_ids: wire.label_ids,
            raw,
        })
    }

    /// One page of `messageAdded` history since `start`.
    pub async fn history(
        &self,
        start: &str,
        page_token: Option<&str>,
        max: u32,
    ) -> Result<HistoryPage, GmailError> {
        let mut query = vec![
            ("startHistoryId", start.to_string()),
            ("historyTypes", "messageAdded".to_string()),
            ("maxResults", max.to_string()),
        ];
        if let Some(token) = page_token {
            query.push(("pageToken", token.to_string()));
        }
        let response = self
            .get(&["history"], &query, On404::HistoryExpired)
            .await?;
        json(response, "the history page").await
    }

    /// `GET {base}/gmail/v1/users/me/{segments}` and the status mapping.
    async fn get(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
        on_404: On404,
    ) -> Result<Response, GmailError> {
        let mut url = Url::parse(self.base)
            .map_err(|_| GmailError::Transport("the Gmail base URL is invalid".into()))?;
        url.path_segments_mut()
            .map_err(|()| GmailError::Transport("the Gmail base URL is invalid".into()))?
            .pop_if_empty()
            .extend(["gmail", "v1", "users", "me"])
            .extend(segments);
        let response = self
            .http
            .get(url)
            .bearer_auth(self.token)
            .query(query)
            .send()
            .await
            .map_err(transport)?;
        check(response, on_404).await
    }
}

fn transport(err: reqwest::Error) -> GmailError {
    GmailError::Transport(err.without_url().to_string())
}

/// Maps a non-2xx answer to its [`GmailError`].
async fn check(response: Response, on_404: On404) -> Result<Response, GmailError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after = || {
        response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
    };
    Err(match status {
        StatusCode::UNAUTHORIZED => GmailError::Unauthorized,
        StatusCode::NOT_FOUND => match on_404 {
            On404::HistoryExpired => GmailError::HistoryExpired,
            On404::NotFound => GmailError::NotFound,
            On404::Http => GmailError::Http { status: 404 },
        },
        StatusCode::TOO_MANY_REQUESTS => GmailError::RateLimited {
            retry_after: retry_after(),
        },
        StatusCode::FORBIDDEN => {
            let retry_after = retry_after();
            let reason = read_capped(response, ERROR_LIMIT)
                .await
                .ok()
                .flatten()
                .and_then(|body| serde_json::from_slice::<ErrorBody>(&body).ok())
                .and_then(|body| body.error.errors.into_iter().next())
                .and_then(|item| item.reason);
            match reason.as_deref() {
                Some("rateLimitExceeded" | "userRateLimitExceeded") => {
                    GmailError::RateLimited { retry_after }
                }
                _ => GmailError::Http { status: 403 },
            }
        }
        other => GmailError::Http {
            status: other.as_u16(),
        },
    })
}

/// The whole body, or `None` once it passes `limit` bytes (the rest is never read).
async fn read_capped(mut response: Response, limit: usize) -> Result<Option<Vec<u8>>, GmailError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if body.len() + chunk.len() > limit {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

async fn json<T: DeserializeOwned>(response: Response, what: &str) -> Result<T, GmailError> {
    let body = read_capped(response, JSON_LIMIT)
        .await?
        .ok_or_else(|| GmailError::Decode(format!("{what} is over {JSON_LIMIT} bytes")))?;
    parse(&body, what)
}

/// Parses JSON without echoing any of its values into the error.
fn parse<T: DeserializeOwned>(body: &[u8], what: &str) -> Result<T, GmailError> {
    serde_json::from_slice(body).map_err(|err| {
        GmailError::Decode(format!(
            "{what}: {:?} error at line {}, column {}",
            err.classify(),
            err.line(),
            err.column()
        ))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const TOKEN: &str = "test-access-token";
    const EML: &[u8] = include_bytes!("../../fixtures/emails/recruiter-outreach.eml");

    fn http() -> reqwest::Client {
        reqwest::Client::new()
    }

    fn raw_body(id: &str, raw: &[u8]) -> serde_json::Value {
        json!({
            "id": id,
            "threadId": format!("t-{id}"),
            "labelIds": ["INBOX", "UNREAD"],
            "raw": URL_SAFE.encode(raw),
            "sizeEstimate": raw.len(),
        })
    }

    #[tokio::test]
    async fn client_reads_fixture_responses_with_get_only() {
        let server = MockServer::start().await;
        let me = "/gmail/v1/users/me";
        Mock::given(path(format!("{me}/profile")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "emailAddress": "me@example.com",
                "messagesTotal": 10,
                "historyId": "1234",
            })))
            .mount(&server)
            .await;
        Mock::given(path(format!("{me}/messages")))
            .and(query_param("q", "newer_than:30d -in:drafts"))
            .and(query_param("maxResults", "2"))
            .and(query_param("pageToken", "p1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "messages": [{"id": "a1", "threadId": "t1"}, {"id": "a2", "threadId": "t2"}],
                "nextPageToken": "p2",
                "resultSizeEstimate": 2,
            })))
            .mount(&server)
            .await;
        Mock::given(path(format!("{me}/messages/a1")))
            .and(query_param("format", "raw"))
            .respond_with(ResponseTemplate::new(200).set_body_json(raw_body("a1", EML)))
            .mount(&server)
            .await;
        // Gmail usually pads `raw`; an unpadded one decodes too.
        Mock::given(path(format!("{me}/messages/a2")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "a2",
                "raw": URL_SAFE_NO_PAD.encode(b"Subject: x\r\n\r\nhi"),
            })))
            .mount(&server)
            .await;
        Mock::given(path(format!("{me}/history")))
            .and(query_param("startHistoryId", "1234"))
            .and(query_param("historyTypes", "messageAdded"))
            .and(query_param("maxResults", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "history": [
                    {"id": "1235", "messages": [{"id": "b1"}]},
                    {"id": "1236", "messagesAdded": [
                        {"message": {"id": "b1", "threadId": "t9", "labelIds": ["INBOX"]}},
                    ]},
                ],
                "historyId": "1240",
            })))
            .mount(&server)
            .await;

        let http = http();
        let uri = server.uri();
        let client = GmailClient::new(&http, &uri, TOKEN);
        assert_eq!(
            client.profile().await.unwrap(),
            Profile {
                email_address: "me@example.com".into(),
                history_id: "1234".into(),
            }
        );
        let list = client
            .list_messages("newer_than:30d -in:drafts", Some("p1"), 2)
            .await
            .unwrap();
        assert_eq!(
            list.messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        assert_eq!(list.next_page_token.as_deref(), Some("p2"));
        let message = client.get_raw("a1", 1 << 20).await.unwrap();
        assert_eq!(message.id, "a1");
        assert_eq!(message.thread_id.as_deref(), Some("t-a1"));
        assert_eq!(message.label_ids, ["INBOX", "UNREAD"]);
        assert_eq!(message.raw, EML);
        assert!(
            !format!("{message:?}").contains("Subject"),
            "Debug omits raw"
        );
        let unpadded = client.get_raw("a2", 1 << 20).await.unwrap();
        assert_eq!(unpadded.raw, b"Subject: x\r\n\r\nhi");
        assert!(unpadded.label_ids.is_empty());
        let history = client.history("1234", None, 5).await.unwrap();
        assert_eq!(history.history_id, "1240");
        assert_eq!(history.next_page_token, None);
        assert_eq!(history.history.len(), 2);
        assert!(history.history[0].messages_added.is_empty());
        assert_eq!(history.history[1].messages_added[0].message.id, "b1");

        let requests = server.received_requests().await.expect("recording is on");
        assert_eq!(requests.len(), 5);
        for request in &requests {
            assert_eq!(request.method.as_str(), "GET", "{}", request.url);
            assert_eq!(
                request
                    .headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok()),
                Some(format!("Bearer {TOKEN}").as_str())
            );
            assert!(request.body.is_empty());
        }
    }

    #[tokio::test]
    async fn client_maps_error_statuses() {
        let reason = |reason: &str| json!({"error": {"code": 403, "errors": [{"reason": reason}]}});
        let cases = [
            (
                "profile",
                ResponseTemplate::new(401),
                GmailError::Unauthorized,
            ),
            (
                "history",
                ResponseTemplate::new(401),
                GmailError::Unauthorized,
            ),
            (
                "history",
                ResponseTemplate::new(404),
                GmailError::HistoryExpired,
            ),
            ("get", ResponseTemplate::new(404), GmailError::NotFound),
            (
                "list",
                ResponseTemplate::new(404),
                GmailError::Http { status: 404 },
            ),
            (
                "get",
                ResponseTemplate::new(429).insert_header("Retry-After", "7"),
                GmailError::RateLimited {
                    retry_after: Some(Duration::from_secs(7)),
                },
            ),
            (
                "list",
                ResponseTemplate::new(429),
                GmailError::RateLimited { retry_after: None },
            ),
            (
                "history",
                ResponseTemplate::new(403).set_body_json(reason("rateLimitExceeded")),
                GmailError::RateLimited { retry_after: None },
            ),
            (
                "get",
                ResponseTemplate::new(403)
                    .insert_header("Retry-After", "3")
                    .set_body_json(reason("userRateLimitExceeded")),
                GmailError::RateLimited {
                    retry_after: Some(Duration::from_secs(3)),
                },
            ),
            (
                "profile",
                ResponseTemplate::new(403).set_body_json(reason("insufficientPermissions")),
                GmailError::Http { status: 403 },
            ),
            (
                "profile",
                ResponseTemplate::new(403).set_body_string("not json"),
                GmailError::Http { status: 403 },
            ),
            (
                "list",
                ResponseTemplate::new(500),
                GmailError::Http { status: 500 },
            ),
            (
                "get",
                ResponseTemplate::new(503),
                GmailError::Http { status: 503 },
            ),
        ];
        let http = http();
        for (endpoint, response, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            let uri = server.uri();
            let client = GmailClient::new(&http, &uri, TOKEN);
            let got = match endpoint {
                "profile" => client.profile().await.map(|_| ()),
                "list" => client.list_messages("q", None, 1).await.map(|_| ()),
                "get" => client.get_raw("m1", 1024).await.map(|_| ()),
                "history" => client.history("1", None, 1).await.map(|_| ()),
                _ => unreachable!(),
            };
            assert_eq!(got, Err(expected.clone()), "{endpoint} → {expected:?}");
            assert!(!got.unwrap_err().to_string().contains(TOKEN));
        }

        // A 2xx body that doesn't parse, and a server that isn't there.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"secret subject\""))
            .mount(&server)
            .await;
        let uri = server.uri();
        let err = GmailClient::new(&http, &uri, TOKEN)
            .profile()
            .await
            .unwrap_err();
        assert!(matches!(err, GmailError::Decode(_)), "{err:?}");
        assert!(!err.to_string().contains("secret"), "{err}");
        let nowhere = "http://127.0.0.1:1";
        let err = GmailClient::new(&http, nowhere, TOKEN)
            .profile()
            .await
            .unwrap_err();
        assert!(matches!(err, GmailError::Transport(_)), "{err:?}");
        assert!(!err.to_string().contains("users/me"), "{err}");
    }

    #[tokio::test]
    async fn client_caps_message_size() {
        let server = MockServer::start().await;
        let max = 1000;
        let me = "/gmail/v1/users/me/messages";
        // Decoded `raw` one byte over the cap.
        Mock::given(path(format!("{me}/over")))
            .respond_with(ResponseTemplate::new(200).set_body_json(raw_body("over", &[b'a'; 1001])))
            .mount(&server)
            .await;
        // Exactly at the cap.
        Mock::given(path(format!("{me}/at")))
            .respond_with(ResponseTemplate::new(200).set_body_json(raw_body("at", &[b'a'; 1000])))
            .mount(&server)
            .await;
        // A small `raw` inside an HTTP body past `max * 4 / 3 + 64 KiB`.
        Mock::given(path(format!("{me}/huge")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "huge",
                "raw": URL_SAFE.encode(b"Subject: x\r\n\r\nhi"),
                "padding": "x".repeat(70 * 1024),
            })))
            .mount(&server)
            .await;

        let http = http();
        let uri = server.uri();
        let client = GmailClient::new(&http, &uri, TOKEN);
        assert_eq!(
            client.get_raw("over", max).await,
            Err(GmailError::Oversized)
        );
        assert_eq!(client.get_raw("at", max).await.unwrap().raw.len(), 1000);
        assert_eq!(
            client.get_raw("huge", max).await,
            Err(GmailError::Oversized)
        );
        // The same body passes under a cap it fits.
        assert!(client.get_raw("huge", 1 << 20).await.is_ok());
    }
}
