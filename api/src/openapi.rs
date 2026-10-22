//! The API's OpenAPI document, built from the `#[utoipa::path]` annotations next to each
//! handler. `api/openapi.json` is this document, and the web app's types are generated
//! from that file. The debug-only `/dev` routes are not part of the contract.

use utoipa::OpenApi;

use crate::events::{self, Event};
use crate::health::{self, Health};
use crate::jobs::{self, Job};

#[derive(OpenApi)]
#[openapi(
    info(title = "Waypoint API"),
    paths(health::health, jobs::show, events::events),
    components(schemas(Health, Job, Event))
)]
struct ApiDoc;

/// The OpenAPI document for every route the API mounts in all builds.
pub fn document() -> utoipa::openapi::OpenApi {
    let mut doc = ApiDoc::openapi();
    // utoipa fills these from Cargo metadata, which this crate leaves empty.
    doc.info.description = None;
    doc.info.license = None;
    doc
}

/// [`document`] as pretty JSON with a trailing newline: the exact contents of
/// `api/openapi.json`.
pub fn to_json() -> String {
    let mut json = document()
        .to_pretty_json()
        .expect("the OpenAPI document serializes");
    json.push('\n');
    json
}
