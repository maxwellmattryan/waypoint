//! The `claude` argv. Every call is isolated: no built-in tools, no MCP servers beyond
//! an explicit config, and no saved session. `--bare` is never passed because it ignores
//! the subscription login and bills the API instead.

use std::ffi::OsString;
use std::path::Path;

use serde_json::Value;

/// The name Waypoint's MCP server is registered under in the generated config.
pub(crate) const MCP_SERVER_NAME: &str = "waypoint";

/// Flags passed on every call, in order. `--tools` takes a literal empty string.
pub(crate) const BASE_ARGS: [&str; 10] = [
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
];

/// Builds the argv: [`BASE_ARGS`], then `--model` and `--json-schema` (compact JSON) when
/// given, then `--mcp-config <path> --allowedTools mcp__waypoint` when an MCP config is
/// given. Both MCP flags are variadic in the CLI, so they come last and nothing follows
/// their values. The prompt is sent on stdin, never here.
pub(crate) fn build(
    model: Option<&str>,
    schema: Option<&Value>,
    mcp_config: Option<&Path>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = BASE_ARGS.iter().map(OsString::from).collect();
    if let Some(model) = model {
        args.push("--model".into());
        args.push(model.into());
    }
    if let Some(schema) = schema {
        args.push("--json-schema".into());
        args.push(schema.to_string().into());
    }
    if let Some(config) = mcp_config {
        args.push("--mcp-config".into());
        args.push(config.into());
        args.push("--allowedTools".into());
        args.push(format!("mcp__{MCP_SERVER_NAME}").into());
    }
    args
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|arg| arg.into_string().expect("utf-8 arg"))
            .collect()
    }

    #[test]
    fn base_args_are_isolated() {
        let args = strings(build(None, None, None));
        assert_eq!(
            args,
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
        assert!(!args.iter().any(|arg| arg == "--bare"));
    }

    #[test]
    fn model_and_schema_are_appended() {
        let schema = json!({"type": "object", "properties": {"word": {"type": "string"}}});
        let args = strings(build(Some("haiku"), Some(&schema), None));
        assert_eq!(&args[..BASE_ARGS.len()], BASE_ARGS);
        let extra = &args[BASE_ARGS.len()..];
        assert_eq!(extra[..3], ["--model", "haiku", "--json-schema"]);
        assert_eq!(extra.len(), 4);
        assert!(!extra[3].contains(char::is_whitespace), "schema is compact");
        let sent: Value = serde_json::from_str(&extra[3]).expect("schema is JSON");
        assert_eq!(sent, schema);

        let only_model = strings(build(Some("opus"), None, None));
        assert_eq!(&only_model[BASE_ARGS.len()..], ["--model", "opus"]);
        assert!(!only_model.iter().any(|arg| arg == "--bare"));
    }

    #[test]
    fn mcp_config_args_come_last() {
        let schema = json!({"type": "object"});
        let config = Path::new("/tmp/waypoint-mcp-x/mcp.json");
        let args = strings(build(Some("haiku"), Some(&schema), Some(config)));
        assert_eq!(&args[..BASE_ARGS.len()], BASE_ARGS);
        assert_eq!(
            &args[BASE_ARGS.len()..],
            [
                "--model",
                "haiku",
                "--json-schema",
                r#"{"type":"object"}"#,
                "--mcp-config",
                "/tmp/waypoint-mcp-x/mcp.json",
                "--allowedTools",
                "mcp__waypoint",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--bare"));

        let plain = strings(build(None, None, Some(config)));
        assert_eq!(
            &plain[BASE_ARGS.len()..],
            [
                "--mcp-config",
                "/tmp/waypoint-mcp-x/mcp.json",
                "--allowedTools",
                "mcp__waypoint"
            ]
        );
    }
}
