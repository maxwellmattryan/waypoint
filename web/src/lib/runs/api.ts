import type { ApiClient, Run, RunDetail } from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';

export type { Result };

/** Runs one `GET /runs` page holds; a full page means there may be more. */
export const PAGE_SIZE = 50;

export interface ListRunsOptions {
	limit?: number;
	/** Only runs with an id below this one: the next page. */
	before?: number;
}

/** Runs newest first, a page at a time. */
export function listRuns(
	client: ApiClient,
	{ limit = PAGE_SIZE, before }: ListRunsOptions = {}
): Promise<Result<Run[]>> {
	const query = before === undefined ? { limit } : { limit, before };
	return run(200, () => client.GET('/runs', { params: { query } }));
}

/** One run with its steps and their tool calls; a 404 is `not-found`. */
export function getRun(client: ApiClient, id: number): Promise<Result<RunDetail>> {
	return run(200, () => client.GET('/runs/{id}', { params: { path: { id } } }));
}
