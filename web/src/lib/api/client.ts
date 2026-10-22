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
