//! The API's OpenAPI document, built from the `#[utoipa::path]` annotations next to each
//! handler. `api/openapi.json` is this document, and the web app's types are generated
//! from that file. The debug-only `/dev` routes are not part of the contract.

use utoipa::OpenApi;

use crate::applications::stages::{self, NewStage, Stage, StageChange, StageOrder};
use crate::applications::timeline::{self, ApplicationEvent, EntryKind, EventKind, NewEntry};
use crate::applications::{
    self, Application, ApplicationInput, ApplicationStatus, FieldError, NewApplication,
    StatusChange, ValidationErrors,
};
use crate::capture::{
    self, Capture, CaptureFailure, CaptureFailureReason, CaptureMethod, CaptureQueued,
    CaptureRequest, CapturedFields,
};
use crate::events::{self, Event};
use crate::google::{self, ConnectionState, GoogleDisconnect, GoogleStatus};
use crate::health::{self, Health};
use crate::jobs::{self, Job};
use crate::resumes::{self, ResumeLink, ResumeVersion, ResumeVersionDetail};
use crate::runs::{
    self, Run, RunDetail, RunStatus, RunStep, RunStepDetail, StepStatus, ToolCall, ToolCallStatus,
};
use crate::tools::Permission;

#[derive(OpenApi)]
#[openapi(
    info(title = "Waypoint API"),
    paths(
        health::health,
        jobs::show,
        events::events,
        google::connect,
        google::callback,
        google::status,
        google::disconnect,
        applications::list,
        applications::create,
        applications::show,
        applications::update,
        applications::set_status,
        applications::set_resume,
        applications::delete,
        resumes::list,
        resumes::upload,
        resumes::show,
        resumes::pdf,
        resumes::delete,
        stages::list,
        stages::add,
        stages::update,
        stages::reorder,
        stages::remove,
        timeline::list_events,
        timeline::add_entry,
        capture::capture,
        runs::list,
        runs::show,
    ),
    components(schemas(
        Health,
        Job,
        Event,
        GoogleStatus,
        ConnectionState,
        GoogleDisconnect,
        Application,
        ApplicationStatus,
        NewApplication,
        ApplicationInput,
        StatusChange,
        Stage,
        NewStage,
        StageChange,
        StageOrder,
        ApplicationEvent,
        EventKind,
        NewEntry,
        EntryKind,
        CaptureRequest,
        Capture,
        CapturedFields,
        CaptureMethod,
        CaptureQueued,
        CaptureFailure,
        CaptureFailureReason,
        ValidationErrors,
        FieldError,
        ResumeVersion,
        ResumeVersionDetail,
        ResumeLink,
        Run,
        RunStatus,
        RunDetail,
        RunStep,
        StepStatus,
        RunStepDetail,
        ToolCall,
        ToolCallStatus,
        Permission,
    ))
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
