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
/** Why a page couldn't be read. */
export type CaptureFailureReason = components['schemas']['CaptureFailureReason'];

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
