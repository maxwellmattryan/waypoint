//! The OpenAPI contract: `api/openapi.json` matches the code, covers every mounted route
//! and describes the wire shapes the handlers produce.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use waypoint_api::claude::ClaudeRunner;
use waypoint_api::openapi;

const COMMITTED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json");
const STALE: &str = "api/openapi.json is stale; run scripts/gen-api.sh";

fn document() -> Value {
    serde_json::to_value(openapi::document()).expect("the document serializes")
}

#[test]
fn openapi_json_is_up_to_date() {
    let committed = std::fs::read_to_string(COMMITTED).unwrap_or_else(|err| {
        panic!("{STALE} (cannot read {COMMITTED}: {err})");
    });
    // `assert!` rather than `assert_eq!` so the hint isn't buried under two full documents.
    assert!(committed == openapi::to_json(), "{STALE}");
}

#[rocket::async_test]
async fn every_mounted_route_is_documented() {
    // The pool is lazy and never connects; building Rocket doesn't touch it.
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1/unused")
        .expect("valid url");
    let rocket = waypoint_api::rocket(
        rocket::Config::figment(),
        pool,
        ClaudeRunner::new("claude", Duration::from_secs(1)),
        waypoint_api::jobs::Config::default(),
        waypoint_api::google::Config::default(),
    );
    let doc = document();
    let paths = doc["paths"].as_object().expect("paths");

    let mut checked = 0;
    for route in rocket.routes() {
        let uri = route.uri.path().to_string();
        if uri.starts_with("/dev") {
            continue;
        }
        let path = uri
            .split('/')
            .map(|segment| match segment.strip_prefix('<') {
                Some(rest) => format!("{{{}}}", rest.trim_end_matches('>')),
                None => segment.to_string(),
            })
            .collect::<Vec<_>>()
            .join("/");
        let method = route.method.as_str().to_lowercase();
        assert!(
            paths
                .get(&path)
                .and_then(|item| item.get(&method))
                .is_some(),
            "{} {uri} is mounted but not in the OpenAPI document; add #[utoipa::path] to its \
             handler, list it in api/src/openapi.rs and run scripts/gen-api.sh",
            route.method,
        );
        checked += 1;
    }
    assert!(checked > 0, "no routes were checked");

    for path in paths.keys() {
        assert!(!path.starts_with("/dev"), "{path} must not be documented");
    }
}

fn property_names(schema: &Value) -> BTreeSet<String> {
    schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect()
}

#[test]
fn document_matches_wire_shapes() {
    let doc = document();
    assert!(doc["openapi"].as_str().unwrap().starts_with("3.1"));
    let schemas = &doc["components"]["schemas"];

    let job = &schemas["Job"];
    let fields = [
        "id",
        "kind",
        "status",
        "attempts",
        "max_attempts",
        "run_at",
        "last_error",
        "result",
        "created_at",
        "updated_at",
        "finished_at",
    ];
    let expected: BTreeSet<String> = fields.iter().map(|f| f.to_string()).collect();
    assert_eq!(
        property_names(job),
        expected,
        "Job fields (payload is skipped)"
    );
    for timestamp in ["run_at", "created_at", "updated_at", "finished_at"] {
        assert_eq!(
            job["properties"][timestamp]["format"], "date-time",
            "{timestamp}"
        );
    }

    let variants = schemas["Event"]["oneOf"]
        .as_array()
        .expect("Event is a union");
    let mut names = BTreeSet::new();
    for variant in variants {
        let tag = &variant["properties"]["type"]["enum"];
        assert_eq!(tag.as_array().map(Vec::len), Some(1), "{variant}");
        let name = tag[0].as_str().unwrap().to_string();
        assert!(
            variant["required"]
                .as_array()
                .unwrap()
                .contains(&"type".into()),
            "{name} requires type"
        );
        let props = property_names(variant);
        let payload: BTreeSet<&str> = props
            .iter()
            .map(String::as_str)
            .filter(|p| *p != "type")
            .collect();
        let want: BTreeSet<&str> = match name.as_str() {
            n if n.starts_with("job.") => ["job"].into(),
            "queue.paused" => ["until"].into(),
            "app.lagged" => ["skipped"].into(),
            _ => BTreeSet::new(),
        };
        assert_eq!(payload, want, "{name} payload");
        names.insert(name);
    }
    let wire_names: BTreeSet<String> = [
        "job.queued",
        "job.started",
        "job.retrying",
        "job.deferred",
        "job.succeeded",
        "job.failed",
        "queue.paused",
        "queue.resumed",
        "app.connected",
        "app.lagged",
    ]
    .iter()
    .map(|n| n.to_string())
    .collect();
    assert_eq!(names, wire_names);

    let paths = &doc["paths"];
    assert!(
        paths["/events"]["get"]["responses"]["200"]["content"]["text/event-stream"].is_object(),
        "/events streams text/event-stream"
    );
    for (path, item) in paths.as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            assert!(
                operation["responses"]["403"].is_object(),
                "{method} {path} documents 403"
            );
        }
    }
    assert!(paths["/jobs/{id}"]["get"]["responses"]["404"].is_object());

    let status = &schemas["GoogleStatus"];
    let fields: BTreeSet<String> = ["configured", "state", "scopes", "connected_at"]
        .iter()
        .map(|f| f.to_string())
        .collect();
    assert_eq!(property_names(status), fields, "GoogleStatus fields");
    let required: BTreeSet<String> = status["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert_eq!(required, fields, "every GoogleStatus field is always sent");
    assert_eq!(
        status["properties"]["connected_at"]["format"], "date-time",
        "connected_at"
    );
    assert_eq!(
        schemas["ConnectionState"]["enum"],
        serde_json::json!(["disconnected", "connected", "reconnect_required"])
    );
    assert_eq!(
        property_names(&schemas["GoogleDisconnect"]),
        BTreeSet::from(["revoked".to_string()])
    );

    let application = &schemas["Application"];
    let fields: BTreeSet<String> = [
        "id",
        "company",
        "title",
        "url",
        "location",
        "comp",
        "source",
        "notes",
        "status",
        "status_changed_at",
        "created_at",
        "updated_at",
    ]
    .iter()
    .map(|f| f.to_string())
    .collect();
    assert_eq!(property_names(application), fields, "Application fields");
    let required: BTreeSet<String> = application["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert_eq!(required, fields, "every Application field is always sent");
    for nullable in ["url", "location", "comp", "source", "notes"] {
        assert_eq!(
            application["properties"][nullable]["type"],
            serde_json::json!(["string", "null"]),
            "{nullable} is a nullable string"
        );
    }
    for timestamp in ["status_changed_at", "created_at", "updated_at"] {
        assert_eq!(
            application["properties"][timestamp]["format"], "date-time",
            "{timestamp}"
        );
    }
    assert_eq!(
        schemas["ApplicationStatus"]["enum"],
        serde_json::json!(["sourced", "applied", "interviewing", "offer", "rejected"])
    );
    for operation in [
        &paths["/applications"]["get"],
        &paths["/applications"]["post"],
        &paths["/applications/{id}"]["get"],
        &paths["/applications/{id}"]["put"],
        &paths["/applications/{id}"]["delete"],
        &paths["/applications/{id}/status"]["put"],
    ] {
        assert!(operation["responses"]["403"].is_object(), "{operation}");
        assert_eq!(operation["tags"], serde_json::json!(["applications"]));
    }
    assert!(paths["/applications"]["post"]["responses"]["201"].is_object());
    assert!(paths["/applications"]["post"]["responses"]["422"].is_object());
    assert!(paths["/applications/{id}"]["delete"]["responses"]["204"].is_object());

    let google = &paths["/auth/google"];
    assert!(google["get"]["responses"]["200"].is_object());
    assert!(google["delete"]["responses"]["404"].is_object());
    let connect = &paths["/auth/google/connect"]["get"]["responses"];
    assert!(connect["303"].is_object(), "connect redirects with 303");
    assert!(
        connect["503"]["content"]["text/plain"].is_object(),
        "connect answers 503 text/plain when not configured"
    );
    assert!(
        paths["/auth/google/callback"]["get"]["responses"]["200"]["content"]["text/html"]
            .is_object(),
        "the callback answers text/html"
    );
}
