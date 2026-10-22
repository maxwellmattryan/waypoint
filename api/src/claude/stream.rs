//! Parses the CLI's `stream-json` output. Only the events Waypoint needs are typed;
//! unknown event types and fields are ignored so CLI upgrades that add events don't break
//! calls.

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{ClaudeError, Usage};

/// Substrings (matched lowercased) that mark a usage-limit message. The CLI's wording
/// varies by version, so the match is deliberately loose and only applied to error
/// results and `rate_limit` assistant errors.
const USAGE_LIMIT_MARKERS: [&str; 3] = ["usage limit", "hit your limit", "limit reached"];

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    System {
        #[serde(default)]
        model: Option<String>,
    },
    Assistant {
        #[serde(default)]
        message: Option<AssistantMessage>,
        #[serde(default)]
        error: Option<String>,
    },
    Result(ResultEvent),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    content: Option<Value>,
}

impl AssistantMessage {
    /// Concatenates the message's text blocks.
    fn text(&self) -> String {
        match &self.content {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResultEvent {
    subtype: String,
    is_error: bool,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    errors: Vec<Value>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default, rename = "modelUsage")]
    model_usage: Option<Map<String, Value>>,
    #[serde(default)]
    structured_output: Option<Value>,
}

/// The final `result` event, reduced to what callers need.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Finished {
    pub subtype: String,
    pub is_error: bool,
    pub message: String,
    pub structured: Option<Value>,
}

/// Everything read from one call's stdout.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Parsed {
    pub model: Option<String>,
    pub usage: Usage,
    pub cost_usd: Option<f64>,
    /// Text of an assistant event with `"error": "rate_limit"` that reads as a usage limit.
    pub limit_message: Option<String>,
    pub result: Option<Finished>,
}

/// A successful call's output.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Completed {
    pub text: String,
    pub structured: Option<Value>,
}

impl Parsed {
    /// Classifies the stream. `None` means the stream ended without a `result` event,
    /// which the caller reports as a process failure.
    pub(crate) fn outcome(&self) -> Option<Result<Completed, ClaudeError>> {
        if let Some(message) = &self.limit_message {
            return Some(Err(ClaudeError::UsageLimit {
                message: message.clone(),
            }));
        }
        let result = self.result.as_ref()?;
        if !result.is_error {
            return Some(Ok(Completed {
                text: result.message.clone(),
                structured: result.structured.clone(),
            }));
        }
        Some(Err(if is_usage_limit(&result.message) {
            ClaudeError::UsageLimit {
                message: result.message.clone(),
            }
        } else {
            ClaudeError::Cli {
                subtype: result.subtype.clone(),
                message: result.message.clone(),
            }
        }))
    }
}

/// Whether an error message reads as a subscription usage limit.
pub(crate) fn is_usage_limit(text: &str) -> bool {
    let text = text.to_lowercase();
    USAGE_LIMIT_MARKERS
        .iter()
        .any(|marker| text.contains(marker))
}

/// Parses stdout lines. A line that isn't JSON, or a known event missing required
/// fields, is a protocol error; blank lines and unknown events are skipped.
pub(crate) fn parse<'a>(lines: impl IntoIterator<Item = &'a str>) -> Result<Parsed, ClaudeError> {
    let mut parsed = Parsed::default();
    let mut assistant_model = None;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event: Event = serde_json::from_str(line).map_err(|err| ClaudeError::Protocol {
            message: format!("cannot parse stdout line ({err}): {}", preview(line)),
        })?;
        match event {
            Event::System { model } => {
                if parsed.model.is_none() {
                    parsed.model = model;
                }
            }
            Event::Assistant { message, error } => {
                if let Some(message) = message {
                    if assistant_model.is_none() {
                        assistant_model = message.model.clone();
                    }
                    if error.as_deref() == Some("rate_limit") {
                        let text = message.text();
                        if is_usage_limit(&text) {
                            parsed.limit_message = Some(text);
                        }
                    }
                }
            }
            Event::Result(result) => {
                if parsed.model.is_none() {
                    parsed.model = assistant_model.take().or_else(|| {
                        result
                            .model_usage
                            .as_ref()
                            .and_then(|models| models.keys().next().cloned())
                    });
                }
                parsed.usage = result.usage.unwrap_or_default();
                parsed.cost_usd = result.total_cost_usd;
                let message = result.result.unwrap_or_else(|| {
                    let errors: Vec<String> = result
                        .errors
                        .iter()
                        .map(|error| match error {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    errors.join("; ")
                });
                parsed.result = Some(Finished {
                    subtype: result.subtype,
                    is_error: result.is_error,
                    message,
                    structured: result.structured_output,
                });
            }
            Event::Other => {}
        }
    }
    if parsed.model.is_none() {
        parsed.model = assistant_model;
    }
    Ok(parsed)
}

/// The first 200 characters of a line, for error messages.
fn preview(line: &str) -> String {
    const MAX: usize = 200;
    match line.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SUCCESS: &str = include_str!("../../tests/fixtures/claude/success.jsonl");

    #[test]
    fn parses_success_stream() {
        let parsed = parse(SUCCESS.lines()).expect("valid stream");
        assert_eq!(parsed.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(
            parsed.usage,
            Usage {
                input_tokens: Some(2),
                output_tokens: Some(4),
                cache_creation_input_tokens: Some(3327),
                cache_read_input_tokens: Some(649),
            }
        );
        assert_eq!(parsed.cost_usd, Some(0.00067409));
        let completed = parsed.outcome().expect("has result").expect("success");
        assert_eq!(completed.text, "pong");
        assert_eq!(completed.structured, None);
    }

    fn result_line(subtype: &str, is_error: bool, text: &str) -> String {
        json!({
            "type": "result",
            "subtype": subtype,
            "is_error": is_error,
            "result": text,
            "total_cost_usd": 0,
            "usage": {"input_tokens": 0, "output_tokens": 0},
        })
        .to_string()
    }

    fn rate_limit_line(text: &str) -> String {
        json!({
            "type": "assistant",
            "error": "rate_limit",
            "message": {
                "model": "<synthetic>",
                "role": "assistant",
                "content": [{"type": "text", "text": text}],
            },
        })
        .to_string()
    }

    #[test]
    fn classifies_error_results() {
        enum Expect {
            UsageLimit,
            Cli(&'static str),
        }
        let rows = [
            (
                vec![result_line(
                    "success",
                    true,
                    "Claude AI usage limit reached|1761000000",
                )],
                Expect::UsageLimit,
            ),
            (
                vec![result_line(
                    "success",
                    true,
                    "You've hit your limit · resets 5pm",
                )],
                Expect::UsageLimit,
            ),
            (
                vec![
                    rate_limit_line("5-hour limit reached ∙ resets 3am"),
                    result_line("success", false, "5-hour limit reached ∙ resets 3am"),
                ],
                Expect::UsageLimit,
            ),
            (
                vec![
                    json!({"type": "result", "subtype": "error_max_turns", "is_error": true})
                        .to_string(),
                ],
                Expect::Cli("error_max_turns"),
            ),
            (
                vec![result_line("success", true, "API Error: 500")],
                Expect::Cli("success"),
            ),
        ];

        for (lines, expect) in rows {
            let parsed = parse(lines.iter().map(String::as_str)).expect("valid stream");
            let err = parsed
                .outcome()
                .expect("has result")
                .expect_err("error result");
            match (expect, err) {
                (Expect::UsageLimit, ClaudeError::UsageLimit { message }) => {
                    assert!(is_usage_limit(&message), "{lines:?}");
                }
                (Expect::Cli(want), ClaudeError::Cli { subtype, .. }) => {
                    assert_eq!(subtype, want, "{lines:?}");
                }
                (_, err) => panic!("{lines:?} classified as {err:?}"),
            }
        }
    }

    #[test]
    fn rejects_garbage_and_ignores_unknown_events() {
        let err = parse(["not json"]).expect_err("garbage");
        assert!(matches!(err, ClaudeError::Protocol { .. }));

        let err =
            parse([r#"{"type":"result","subtype":"success"}"#]).expect_err("missing is_error");
        assert!(matches!(err, ClaudeError::Protocol { .. }));

        let parsed = parse([
            r#"{"type":"stream_event","event":{}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
            "",
        ])
        .expect("unknown events are ignored");
        assert_eq!(parsed.outcome(), None);
    }
}
