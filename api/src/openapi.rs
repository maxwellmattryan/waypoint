//! The API's OpenAPI document, built from the `#[utoipa::path]` annotations next to each
//! handler. `api/openapi.json` is this document, and the web app's types are generated
//! from that file. The debug-only `/dev` routes are not part of the contract.

use utoipa::OpenApi;

use crate::applications::stages::{self, NewStage, Stage, StageChange, StageOrder};
use crate::applications::timeline::{
    self, ApplicationEvent, EntryKind, EventKind, NewEntry, SourceEmail,
};
use crate::applications::{
    self, Application, ApplicationInput, ApplicationStatus, FieldError, NewApplication,
    StatusChange, ValidationErrors,
};
use crate::capture::{
    self, Capture, CaptureFailure, CaptureFailureReason, CaptureMethod, CaptureQueued,
    CaptureRequest, CapturedFields,
};
use crate::events::{self, Event};
use crate::gmail::{self, GmailSyncQueued, GmailSyncStatus, SyncActivity, SyncCounts, SyncMode};
use crate::google::{self, ConnectionState, GoogleDisconnect, GoogleRevocation, GoogleStatus};
use crate::health::{self, Health};
use crate::jobs::{self, Job};
use crate::kill_switch::{self, DataDeletion, DeleteAllData, KillSwitch, SetKillSwitch};
use crate::memories::{
    self, Memory, MemoryInput, MemoryKind, MemoryStatus, MemoryTopic, SourceKind,
};
use crate::memory_signals::{self, MemorySignal, SignalChange, SignalKind, SignalOp};
use crate::profile::ProfileImportSummary;
use crate::profile::linkedin;
use crate::profile::resume::{self as resume_profile, ProfileImportQueued};
use crate::resumes::{self, ResumeLink, ResumeVersion, ResumeVersionDetail};
use crate::runs::actions::{self, AgentAction};
use crate::runs::approvals::{self, ApproveAction, PendingAction, PendingActionStatus};
use crate::runs::review::{
    self, AcceptReviewItem, DismissReviewItem, ReviewItem, ReviewItemStatus,
};
use crate::runs::{
    self, Run, RunDetail, RunMemory, RunStatus, RunStep, RunStepDetail, StepStatus, ToolCall,
    ToolCallStatus,
};
use crate::sourcing::postings::{self, JobPosting, JobPostingDetail};
use crate::sourcing::{self, Source, SourceBoardQueued, SourceBoardRequest};
use crate::tools::Permission;
use crate::workflows::memory_writer::{self, MemoryWriterQueued};
use crate::workflows::tidy::{self, TidyRequest};

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
        gmail::status,
        gmail::sync_now,
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
        memories::list,
        memories::create,
        memories::show,
        memories::evidence,
        memories::update,
        memories::confirm,
        memories::delete,
        memory_writer::learn,
        memory_signals::list,
        linkedin::import,
        resume_profile::start,
        stages::list,
        stages::add,
        stages::update,
        stages::reorder,
        stages::remove,
        timeline::list_events,
        timeline::add_entry,
        capture::capture,
        sourcing::source_board,
        postings::list,
        postings::show,
        runs::list,
        runs::show,
        approvals::list,
        approvals::approve_route,
        approvals::reject_route,
        actions::list,
        actions::undo_route,
        review::list,
        review::dismiss_route,
        review::accept_route,
        tidy::start_tidy,
        kill_switch::show,
        kill_switch::update,
        kill_switch::delete_all,
    ),
    components(schemas(
        Health,
        Job,
        Event,
        GoogleStatus,
        ConnectionState,
        GoogleDisconnect,
        GmailSyncStatus,
        GmailSyncQueued,
        SyncActivity,
        SyncMode,
        SyncCounts,
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
        SourceEmail,
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
        SourceBoardRequest,
        SourceBoardQueued,
        Source,
        JobPosting,
        JobPostingDetail,
        ValidationErrors,
        FieldError,
        ResumeVersion,
        ResumeVersionDetail,
        ResumeLink,
        Memory,
        MemoryInput,
        MemoryKind,
        MemoryTopic,
        MemoryStatus,
        SourceKind,
        MemorySignal,
        SignalKind,
        SignalOp,
        SignalChange,
        MemoryWriterQueued,
        ProfileImportSummary,
        ProfileImportQueued,
        Run,
        RunStatus,
        RunDetail,
        RunStep,
        StepStatus,
        RunStepDetail,
        RunMemory,
        ToolCall,
        ToolCallStatus,
        Permission,
        PendingAction,
        PendingActionStatus,
        ApproveAction,
        AgentAction,
        ReviewItem,
        ReviewItemStatus,
        DismissReviewItem,
        AcceptReviewItem,
        TidyRequest,
        KillSwitch,
        SetKillSwitch,
        DeleteAllData,
        DataDeletion,
        GoogleRevocation,
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
