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
