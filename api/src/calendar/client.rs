//! A Google Calendar REST client with one call: `POST
//! /calendar/v3/calendars/primary/events?sendUpdates=none` with a `Bearer` token. Only the
//! new event's `id` and `htmlLink` are read back. Errors never carry the token, a URL, the
//! summary or the description.

use std::fmt;

use reqwest::{Response, StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};

/// Largest response body read.
const BODY_LIMIT: usize = 64 * 1024;

/// Why a Calendar call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarError {
    /// HTTP 401: the access token was refused. Google didn't process the request.
    Unauthorized,
    /// HTTP 403, e.g. the grant lacks `calendar.events` or the API isn't enabled.
    Forbidden,
    /// HTTP 429.
    RateLimited,
    /// Any other non-2xx answer.
    Http { status: u16 },
    /// Network error or timeout.
    Transport(String),
    /// A 2xx body that didn't parse or has no event id.
    Decode(String),
}

impl fmt::Display for CalendarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => {
                f.write_str("Google Calendar refused the access token (HTTP 401)")
            }
            Self::Forbidden => f.write_str("Google Calendar refused the request (HTTP 403)"),
            Self::RateLimited => f.write_str("Google Calendar's rate limit was hit (HTTP 429)"),
            Self::Http { status } => write!(f, "Google Calendar answered HTTP {status}"),
            Self::Transport(why) => write!(f, "cannot reach Google Calendar: {why}"),
            Self::Decode(why) => write!(f, "Google Calendar's response did not parse: {why}"),
        }
    }
}

impl std::error::Error for CalendarError {}

/// The event to create. `start` and `end` are RFC 3339; `time_zone` is an IANA name or
/// `UTC`, and is left out of the request when `None`. Its `Debug` omits the summary and the
/// description.
#[derive(Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub summary: String,
    pub description: String,
    pub start: String,
    pub end: String,
    pub time_zone: Option<String>,
}

impl fmt::Debug for NewEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewEvent")
            .field(
                "summary",
                &format_args!("<{} chars>", self.summary.chars().count()),
            )
            .field(
                "description",
                &format_args!("<{} chars>", self.description.chars().count()),
            )
            .field("start", &self.start)
            .field("end", &self.end)
            .field("time_zone", &self.time_zone)
            .finish()
    }
}

impl NewEvent {
    /// The `events.insert` body: no attendees, location or conference data.
    fn body(&self) -> Value {
        let time = |at: &str| match &self.time_zone {
            Some(zone) => json!({"dateTime": at, "timeZone": zone}),
            None => json!({"dateTime": at}),
        };
        json!({
            "summary": self.summary,
            "description": self.description,
            "start": time(&self.start),
            "end": time(&self.end),
            "reminders": {"useDefault": true},
        })
    }
}

/// What Google says about the created event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedEvent {
    pub id: String,
    /// The event's page in Google Calendar; `None` unless it is an `https://` URL.
    pub html_link: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    html_link: Option<String>,
}

/// The Calendar API for one access token. No `Debug`, so the token can't reach a log.
pub struct CalendarClient<'a> {
    http: &'a reqwest::Client,
    base: &'a str,
    token: &'a str,
}

impl<'a> CalendarClient<'a> {
    /// `base` is the API origin, e.g. `https://www.googleapis.com`.
    pub fn new(http: &'a reqwest::Client, base: &'a str, token: &'a str) -> Self {
        Self { http, base, token }
    }

    /// Creates `event` on the `primary` calendar without notifying anyone.
    pub async fn insert_event(&self, event: &NewEvent) -> Result<CreatedEvent, CalendarError> {
        let invalid = || CalendarError::Transport("the Calendar base URL is invalid".into());
        let mut url = Url::parse(self.base).map_err(|_| invalid())?;
        url.path_segments_mut()
            .map_err(|()| invalid())?
            .pop_if_empty()
            .extend(["calendar", "v3", "calendars", "primary", "events"]);
        url.query_pairs_mut().append_pair("sendUpdates", "none");
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token)
            .json(&event.body())
            .send()
            .await
            .map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status {
                StatusCode::UNAUTHORIZED => CalendarError::Unauthorized,
                StatusCode::FORBIDDEN => CalendarError::Forbidden,
                StatusCode::TOO_MANY_REQUESTS => CalendarError::RateLimited,
                other => CalendarError::Http {
                    status: other.as_u16(),
                },
            });
        }
        let body = read_capped(response, BODY_LIMIT).await?.ok_or_else(|| {
            CalendarError::Decode(format!("the event is over {BODY_LIMIT} bytes"))
        })?;
        let wire = parse(&body)?;
        let id = wire
            .id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| CalendarError::Decode("the response has no event id".into()))?;
        Ok(CreatedEvent {
            id,
            html_link: wire.html_link.filter(|link| link.starts_with("https://")),
        })
    }
}

fn transport(err: reqwest::Error) -> CalendarError {
    CalendarError::Transport(err.without_url().to_string())
}

/// The whole body, or `None` once it passes `limit` bytes (the rest is never read).
async fn read_capped(
    mut response: Response,
    limit: usize,
) -> Result<Option<Vec<u8>>, CalendarError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if body.len() + chunk.len() > limit {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

/// Parses the event without echoing any of its values into the error.
fn parse(body: &[u8]) -> Result<Wire, CalendarError> {
    serde_json::from_slice(body).map_err(|err| {
        CalendarError::Decode(format!(
            "the event: {:?} error at line {}, column {}",
            err.classify(),
            err.line(),
            err.column()
        ))
    })
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const TOKEN: &str = "test-access-token";
    const EVENTS: &str = "/calendar/v3/calendars/primary/events";
    const SUMMARY: &str = "Interview: Hooli — Technical interview";
    const DESCRIPTION: &str = "Role: Backend Engineer\nAdded by Waypoint after your approval.";

    fn event(time_zone: Option<&str>) -> NewEvent {
        NewEvent {
            summary: SUMMARY.into(),
            description: DESCRIPTION.into(),
            start: "2030-11-03T19:00:00Z".into(),
            end: "2030-11-03T19:45:00Z".into(),
            time_zone: time_zone.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn client_posts_event_without_attendees() {
        let cases = [
            (
                Some("America/New_York"),
                json!({"id": "evt1", "htmlLink": "https://www.google.com/calendar/event?eid=evt1",
                       "attendees": [{"email": "someone@example.com"}], "status": "confirmed"}),
                json!({
                    "summary": SUMMARY,
                    "description": DESCRIPTION,
                    "start": {"dateTime": "2030-11-03T19:00:00Z", "timeZone": "America/New_York"},
                    "end": {"dateTime": "2030-11-03T19:45:00Z", "timeZone": "America/New_York"},
                    "reminders": {"useDefault": true},
                }),
                CreatedEvent {
                    id: "evt1".into(),
                    html_link: Some("https://www.google.com/calendar/event?eid=evt1".into()),
                },
            ),
            (
                None,
                json!({"id": "evt2", "htmlLink": "http://www.google.com/calendar/event?eid=evt2"}),
                json!({
                    "summary": SUMMARY,
                    "description": DESCRIPTION,
                    "start": {"dateTime": "2030-11-03T19:00:00Z"},
                    "end": {"dateTime": "2030-11-03T19:45:00Z"},
                    "reminders": {"useDefault": true},
                }),
                CreatedEvent {
                    id: "evt2".into(),
                    html_link: None,
                },
            ),
        ];
        let http = reqwest::Client::new();
        for (zone, response, sent, created) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path(EVENTS))
                .and(query_param("sendUpdates", "none"))
                .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(response))
                .mount(&server)
                .await;
            let uri = server.uri();
            let got = CalendarClient::new(&http, &uri, TOKEN)
                .insert_event(&event(zone))
                .await
                .expect("the event is created");
            assert_eq!(got, created);

            let requests = server.received_requests().await.expect("recording is on");
            assert_eq!(requests.len(), 1, "{zone:?}");
            let request = &requests[0];
            assert_eq!(request.method.as_str(), "POST");
            assert_eq!(request.url.path(), EVENTS);
            assert_eq!(request.url.query(), Some("sendUpdates=none"));
            let body: Value = serde_json::from_slice(&request.body).expect("a JSON body");
            assert_eq!(body, sent, "{zone:?}");
            assert!(body.get("attendees").is_none());
        }
        assert!(!format!("{:?}", event(None)).contains("Hooli"));
    }

    #[tokio::test]
    async fn client_maps_error_statuses() {
        let cases = [
            (ResponseTemplate::new(401), CalendarError::Unauthorized),
            (
                ResponseTemplate::new(403).set_body_json(
                    json!({"error": {"code": 403, "message": format!("no: {SUMMARY}")}}),
                ),
                CalendarError::Forbidden,
            ),
            (ResponseTemplate::new(429), CalendarError::RateLimited),
            (
                ResponseTemplate::new(500),
                CalendarError::Http { status: 500 },
            ),
        ];
        let http = reqwest::Client::new();
        let mut seen = Vec::new();
        for (response, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(response)
                .mount(&server)
                .await;
            let uri = server.uri();
            let got = CalendarClient::new(&http, &uri, TOKEN)
                .insert_event(&event(Some("UTC")))
                .await;
            assert_eq!(got, Err(expected.clone()), "{expected:?}");
            seen.push(expected.to_string());
        }

        // A 2xx body without an id, one that doesn't parse, and a server that isn't there.
        for body in [
            json!({"htmlLink": "https://www.google.com/calendar/event"}),
            json!({"id": ""}),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            let uri = server.uri();
            let err = CalendarClient::new(&http, &uri, TOKEN)
                .insert_event(&event(None))
                .await
                .unwrap_err();
            assert!(matches!(err, CalendarError::Decode(_)), "{err:?}");
            seen.push(err.to_string());
        }
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!("{{\"{SUMMARY}\"")))
            .mount(&server)
            .await;
        let uri = server.uri();
        let err = CalendarClient::new(&http, &uri, TOKEN)
            .insert_event(&event(None))
            .await
            .unwrap_err();
        assert!(matches!(err, CalendarError::Decode(_)), "{err:?}");
        seen.push(err.to_string());
        let err = CalendarClient::new(&http, "http://127.0.0.1:1", TOKEN)
            .insert_event(&event(None))
            .await
            .unwrap_err();
        assert!(matches!(err, CalendarError::Transport(_)), "{err:?}");
        seen.push(err.to_string());

        let distinct: std::collections::BTreeSet<&String> = seen[..4].iter().collect();
        assert_eq!(distinct.len(), 4, "{seen:?}");
        for text in &seen {
            for secret in [
                TOKEN,
                "Hooli",
                "Backend Engineer",
                "calendars/primary",
                "127.0.0.1",
            ] {
                assert!(!text.contains(secret), "{text} contains {secret}");
            }
        }
    }
}
