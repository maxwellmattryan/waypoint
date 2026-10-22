//! A small JSON Schema subset for tool inputs and outputs, checked in-repo rather than with a
//! schema crate. Supported keywords: `type` (one name or an array of them), `properties`,
//! `required`, `additionalProperties` (a boolean), `items`, `enum`, `description` and
//! `title`. Anything else is refused when a tool registers.
//!
//! Error messages name the JSON path and what was expected, such as
//! `$.items[2].title: expected string`, and never quote the offending value.

use serde_json::{Map, Value};

/// The names `type` accepts.
const TYPES: [&str; 7] = [
    "object", "array", "string", "integer", "number", "boolean", "null",
];

/// Checks that `schema` uses only the supported keywords, each with a well-formed value.
pub fn check_supported(schema: &Value) -> Result<(), String> {
    check_at(schema, "$")
}

fn check_at(schema: &Value, path: &str) -> Result<(), String> {
    let Value::Object(schema) = schema else {
        return Err(format!("{path}: a schema must be an object"));
    };
    for (keyword, value) in schema {
        let ok = match keyword.as_str() {
            "type" => type_names(value).is_some(),
            "properties" => {
                let Value::Object(properties) = value else {
                    return Err(format!("{path}.properties: expected an object of schemas"));
                };
                for (name, property) in properties {
                    check_at(property, &format!("{path}.properties.{name}"))?;
                }
                true
            }
            "required" => value
                .as_array()
                .is_some_and(|names| names.iter().all(Value::is_string)),
            "additionalProperties" => value.is_boolean(),
            "items" => {
                check_at(value, &format!("{path}.items"))?;
                true
            }
            "enum" => value.as_array().is_some_and(|options| !options.is_empty()),
            "description" | "title" => value.is_string(),
            other => return Err(format!("{path}: unsupported keyword {other:?}")),
        };
        if !ok {
            return Err(format!("{path}.{keyword}: malformed value"));
        }
    }
    Ok(())
}

/// The type names of a `type` value, or `None` when it isn't one known name or a non-empty
/// array of them.
fn type_names(value: &Value) -> Option<Vec<&str>> {
    let names = match value {
        Value::String(name) => vec![name.as_str()],
        Value::Array(names) if !names.is_empty() => names
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    names
        .iter()
        .all(|name| TYPES.contains(name))
        .then_some(names)
}

/// Checks `value` against `schema`, which should have passed [`check_supported`]. Returns
/// the first mismatch found.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    validate_at(schema, value, "$")
}

fn validate_at(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Value::Object(schema) = schema else {
        return Ok(());
    };
    if let Some(names) = schema
        .get("type")
        .and_then(type_names)
        .filter(|names| !names.iter().any(|name| is_type(name, value)))
    {
        return Err(format!("{path}: expected {}", names.join(" or ")));
    }
    if matches!(schema.get("enum"), Some(Value::Array(options)) if !options.contains(value)) {
        return Err(format!("{path}: expected one of the allowed values"));
    }
    match value {
        Value::Object(fields) => validate_object(schema, fields, path),
        Value::Array(items) => match schema.get("items") {
            Some(item) => items
                .iter()
                .enumerate()
                .try_for_each(|(i, value)| validate_at(item, value, &format!("{path}[{i}]"))),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

fn validate_object(
    schema: &Map<String, Value>,
    fields: &Map<String, Value>,
    path: &str,
) -> Result<(), String> {
    if let Some(Value::Array(required)) = schema.get("required") {
        for name in required.iter().filter_map(Value::as_str) {
            if !fields.contains_key(name) {
                return Err(format!("{path}.{name}: required"));
            }
        }
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    for (name, value) in fields {
        match properties.and_then(|properties| properties.get(name)) {
            Some(property) => validate_at(property, value, &format!("{path}.{name}"))?,
            None if closed => return Err(format!("{path}.{name}: not allowed")),
            None => {}
        }
    }
    Ok(())
}

fn is_type(name: &str, value: &Value) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value
                    .as_f64()
                    .is_some_and(|n| n.is_finite() && n.fract() == 0.0)
        }
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn validate_cases() {
        let posting = json!({
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "The role"},
                "level": {"type": ["integer", "null"]},
                "remote": {"type": "boolean"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "status": {"enum": ["open", "closed"]},
                "salary": {
                    "type": "object",
                    "properties": {"min": {"type": "number"}},
                    "required": ["min"],
                    "additionalProperties": false
                },
                "items": {
                    "type": "array",
                    "items": {"type": "object", "properties": {"title": {"type": "string"}}}
                }
            },
            "required": ["title"],
            "additionalProperties": false
        });
        check_supported(&posting).expect("the test schema is supported");

        let ok = [
            json!({"title": "Engineer"}),
            json!({"title": "Engineer", "level": null}),
            json!({"title": "Engineer", "level": 3}),
            json!({"title": "Engineer", "level": 3.0}),
            json!({"title": "Engineer", "tags": [], "remote": false}),
            json!({"title": "Engineer", "status": "open"}),
            json!({"title": "Engineer", "salary": {"min": 1.5}}),
            json!({"title": "Engineer", "items": [{"title": "a"}, {}]}),
        ];
        for value in ok {
            assert_eq!(validate(&posting, &value), Ok(()), "{value}");
        }

        let bad = [
            (json!("s3cret"), "$: expected object"),
            (json!({}), "$.title: required"),
            (json!({"title": 7}), "$.title: expected string"),
            (
                json!({"title": "x", "level": "s3cret"}),
                "$.level: expected integer or null",
            ),
            (
                json!({"title": "x", "level": 1.5}),
                "$.level: expected integer or null",
            ),
            (
                json!({"title": "x", "tags": ["a", 1]}),
                "$.tags[1]: expected string",
            ),
            (
                json!({"title": "x", "status": "s3cret"}),
                "$.status: expected one of the allowed values",
            ),
            (json!({"title": "x", "extra": 1}), "$.extra: not allowed"),
            (
                json!({"title": "x", "salary": {}}),
                "$.salary.min: required",
            ),
            (
                json!({"title": "x", "salary": {"min": 1, "max": 2}}),
                "$.salary.max: not allowed",
            ),
            (
                json!({"title": "x", "salary": {"min": "s3cret"}}),
                "$.salary.min: expected number",
            ),
            (
                json!({"title": "x", "items": [{}, {}, {"title": false}]}),
                "$.items[2].title: expected string",
            ),
        ];
        for (value, expected) in bad {
            let err = validate(&posting, &value).expect_err(&value.to_string());
            assert_eq!(err, expected, "{value}");
            assert!(!err.contains("s3cret"), "{err} quotes the value");
        }

        // An object without `additionalProperties: false` accepts unknown fields.
        let open = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        assert_eq!(validate(&open, &json!({"b": 1})), Ok(()));
        // An empty schema accepts anything.
        assert_eq!(validate(&json!({}), &json!([1, "two", null])), Ok(()));

        let unsupported = [
            (
                json!({"type": "object", "oneOf": []}),
                "$: unsupported keyword \"oneOf\"",
            ),
            (
                json!({"properties": {"a": {"minLength": 1}}}),
                "$.properties.a: unsupported keyword \"minLength\"",
            ),
            (
                json!({"items": {"$ref": "#/x"}}),
                "$.items: unsupported keyword \"$ref\"",
            ),
            (json!({"type": "date"}), "$.type: malformed value"),
            (json!({"type": []}), "$.type: malformed value"),
            (
                json!({"additionalProperties": {}}),
                "$.additionalProperties: malformed value",
            ),
            (json!({"required": [1]}), "$.required: malformed value"),
            (json!(true), "$: a schema must be an object"),
        ];
        for (schema, expected) in unsupported {
            assert_eq!(check_supported(&schema), Err(expected.into()), "{schema}");
        }
    }
}
