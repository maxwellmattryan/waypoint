import createClient from 'openapi-fetch';
import { apiBaseUrl } from '#lib/config.js';
import type { components, paths } from './schema';

/** The `GET /health` body. */
export type Health = components['schemas']['Health'];
/** A job as `GET /jobs/{id}` and the `job.*` events return it. */
export type Job = components['schemas']['Job'];
/** One server-sent event from `GET /events`, discriminated by `type`. */
export type Event = components['schemas']['Event'];
/** The `GET /auth/google` body: whether Google is configured and connected. */
export type GoogleStatus = components['schemas']['GoogleStatus'];
/** One tracked job application, as every `/applications` route returns it. */
export type Application = components['schemas']['Application'];
/** Where an application is in the process. */
export type ApplicationStatus = components['schemas']['ApplicationStatus'];
/** The `POST /applications` body. */
export type NewApplication = components['schemas']['NewApplication'];
/** The `PUT /applications/{id}` body: the text fields, replacing the stored ones. */
export type ApplicationInput = components['schemas']['ApplicationInput'];
/** The 400 and 422 body of the applications routes. */
export type ValidationErrors = components['schemas']['ValidationErrors'];
/** One uploaded resume PDF, without its text, as `GET /resumes` lists it. */
export type ResumeVersion = components['schemas']['ResumeVersion'];
/** One resume version with its extracted text, as `GET /resumes/{id}` returns it. */
export type ResumeVersionDetail = components['schemas']['ResumeVersionDetail'];
/** The `PUT /applications/{id}/resume` body; `null` clears the link. */
export type ResumeLink = components['schemas']['ResumeLink'];
/** One failing field in {@link ValidationErrors}. */
export type FieldError = components['schemas']['FieldError'];
/** One interview stage of an application, as every stage route returns it. */
export type Stage = components['schemas']['Stage'];
/** One entry in an application's timeline. */
export type ApplicationEvent = components['schemas']['ApplicationEvent'];
/** What a timeline entry records. */
export type EventKind = components['schemas']['EventKind'];
/** The kinds of timeline entry the user writes directly. */
export type EntryKind = components['schemas']['EntryKind'];
/** The `POST /captures` body. */
export type CaptureRequest = components['schemas']['CaptureRequest'];
/** A job posting's fields: the 200 body of `POST /captures` and a `capture_extract` job's result. */
export type Capture = components['schemas']['Capture'];
/** The fields of a {@link Capture}, ready to prefill the add form. */
export type CapturedFields = components['schemas']['CapturedFields'];
/** How a {@link Capture} was found. */
export type CaptureMethod = components['schemas']['CaptureMethod'];
/** The 202 body of `POST /captures`: Claude reads the page in this job. */
export type CaptureQueued = components['schemas']['CaptureQueued'];
/** The 502 body of `POST /captures`: why the page couldn't be read. */
export type CaptureFailure = components['schemas']['CaptureFailure'];
/** One agent run, as `GET /runs` lists it and `run.updated` carries it. */
export type Run = components['schemas']['Run'];
/** The `GET /runs/{id}` body: the run with its steps, their tool calls and its memories. */
export type RunDetail = components['schemas']['RunDetail'];
/** One step of a run with its tool calls. */
export type RunStepDetail = components['schemas']['RunStepDetail'];
/** A memory a run's step retrieved, with a snapshot of it at retrieval time. */
export type RunMemory = components['schemas']['RunMemory'];
/** One tool call a step made. */
export type ToolCall = components['schemas']['ToolCall'];
/** Where a run is. */
export type RunStatus = components['schemas']['RunStatus'];
/** Where a step is. */
export type StepStatus = components['schemas']['StepStatus'];
/** How a tool call ended. */
export type ToolCallStatus = components['schemas']['ToolCallStatus'];
/** A tool's permission level. */
export type Permission = components['schemas']['Permission'];
/** Why a page couldn't be read. */
export type CaptureFailureReason = components['schemas']['CaptureFailureReason'];
/** The kill switch, from `GET /kill-switch`. */
export type KillSwitch = components['schemas']['KillSwitch'];
/** The `DELETE /data` body. */
export type DataDeletion = components['schemas']['DataDeletion'];
/** What happened to the Google grant when all data was deleted. */
export type GoogleRevocation = components['schemas']['GoogleRevocation'];
/** An `external` tool call waiting for, or past, the user's decision. */
export type PendingAction = components['schemas']['PendingAction'];
/** One action-log entry: a `write` tool call that changed Waypoint's data. */
export type AgentAction = components['schemas']['AgentAction'];
/** A result a step wasn't confident enough to apply. */
export type ReviewItem = components['schemas']['ReviewItem'];
/** The email that caused a timeline event. */
export type SourceEmail = components['schemas']['SourceEmail'];
/** One learning signal: an approval, rejection or dismissal the user made. */
export type MemorySignal = components['schemas']['MemorySignal'];
/** What decision a learning signal records. */
export type SignalKind = components['schemas']['SignalKind'];
/** One entry of an approval's diff from the drafted to the approved input. */
export type SignalChange = components['schemas']['SignalChange'];
/** The `POST /applications/{id}/tidy` body that starts the sample workflow. */
export type TidyRequest = components['schemas']['TidyRequest'];
/** One thing Waypoint knows about the user, as every `/memories` route returns it. */
export type Memory = components['schemas']['Memory'];
/** The `POST /memories` and `PUT /memories/{id}` body. */
export type MemoryInput = components['schemas']['MemoryInput'];
/** What sort of thing a memory is. */
export type MemoryKind = components['schemas']['MemoryKind'];
/** What a memory is about; which topics are allowed depends on the kind. */
export type MemoryTopic = components['schemas']['MemoryTopic'];
/** Whether the user has confirmed a memory. */
export type MemoryStatus = components['schemas']['MemoryStatus'];
/** Where a memory came from. */
export type SourceKind = components['schemas']['SourceKind'];
/** A public job board Waypoint sources: `greenhouse`, `lever` or `ashby`. */
export type Source = components['schemas']['Source'];
/** One sourced job posting without its description, as `GET /postings` lists it. */
export type JobPosting = components['schemas']['JobPosting'];
/** The `GET /postings/{id}` body: the posting with its plain-text description. */
export type JobPostingDetail = components['schemas']['JobPostingDetail'];
/** How well a fit score fits: `strong`, `partial` or `weak`. */
export type FitBand = components['schemas']['FitBand'];
/** One stored fit score with its cited reasons. */
export type FitScore = components['schemas']['FitScore'];
/** One reason of a fit score, `for` or `against`. */
export type FitReason = components['schemas']['FitReason'];
/** One citation of a fit reason: a memory snapshot or a posting excerpt. */
export type FitCitation = components['schemas']['FitCitation'];
/** The latest `score_fit` run for a posting. */
export type FitRun = components['schemas']['FitRun'];
/** The `GET /postings/{id}/fit` body. */
export type PostingFit = components['schemas']['PostingFit'];
/** The `POST /postings/{id}/fit` 202 body: the scoring run, new or reused. */
export type ScoreFitQueued = components['schemas']['ScoreFitQueued'];
/** One of the fixed application questions a packet answers. */
export type PacketQuestion = components['schemas']['PacketQuestion'];
/** Where a packet is: `ready`, `in_review` or `dismissed`. */
export type PacketStatus = components['schemas']['PacketStatus'];
/** One cited reason for a packet's resume choice. */
export type PacketReason = components['schemas']['PacketReason'];
/** One of a packet's answers; `answer` is `null` when it is missing. */
export type PacketAnswer = components['schemas']['PacketAnswer'];
/** One application packet with its posting's company and title. */
export type ApplicationPacket = components['schemas']['ApplicationPacket'];
/** The `GET /postings/{id}/packets` body. */
export type PostingPackets = components['schemas']['PostingPackets'];
/** The `POST /postings/{id}/packet` 202 body: the prep run, new or reused. */
export type PacketQueued = components['schemas']['PacketQueued'];
/** The `POST /sourcing/boards` 202 body: the board's fetch job, new or reused. */
export type SourceBoardQueued = components['schemas']['SourceBoardQueued'];
/** What a profile import did: the LinkedIn import's body and the resume import job's `result`. */
export type ProfileImportSummary = components['schemas']['ProfileImportSummary'];
/** The `POST /resumes/{id}/profile` body: the import job, new or reused. */
export type ProfileImportQueued = components['schemas']['ProfileImportQueued'];
/** The `GET /gmail/sync` body: whether a Gmail sync is pending and the last run's outcome. */
export type GmailSyncStatus = components['schemas']['GmailSyncStatus'];
/** The `POST /gmail/sync` body: the sync's job, new or reused. */
export type GmailSyncQueued = components['schemas']['GmailSyncQueued'];
/** How a Gmail sync run reads the mailbox. */
export type SyncMode = components['schemas']['SyncMode'];

export type ApiClient = ReturnType<typeof createClient<paths>>;

export interface ApiClientOptions {
	/** API origin. Defaults to {@link apiBaseUrl}. */
	baseUrl?: string;
	/** Replaces the global `fetch`, e.g. with a stub in tests. */
	fetch?: typeof fetch;
}

/**
 * A client typed by the generated OpenAPI `paths`: request paths, params and
 * response bodies all come from `api/openapi.json`.
 */
export function createApiClient(options: ApiClientOptions = {}): ApiClient {
	return createClient<paths>({
		baseUrl: options.baseUrl ?? apiBaseUrl(),
		...(options.fetch ? { fetch: options.fetch } : {})
	});
}
