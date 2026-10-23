import type { ApiClient, JobPosting, JobPostingDetail, Source, SourceBoardQueued } from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';

export type { Result };

/** Postings one `GET /postings` page holds; a full page means there may be more. */
export const PAGE_SIZE = 50;

/** The board providers, in the order the filters and the dialog list them. */
export const SOURCES: readonly Source[] = ['greenhouse', 'lever', 'ashby'];

export const SOURCE_LABELS: Record<Source, string> = {
	greenhouse: 'Greenhouse',
	lever: 'Lever',
	ashby: 'Ashby'
};

/** The page's filters. Blank text, `''` and `false` mean "any". */
export interface PostingFilters {
	company: string;
	location: string;
	source: Source | '';
	/** `remote` for only remote postings, `onsite` for only non-remote ones. */
	remote: '' | 'remote' | 'onsite';
	includeRemoved: boolean;
}

export const EMPTY_FILTERS: PostingFilters = {
	company: '',
	location: '',
	source: '',
	remote: '',
	includeRemoved: false
};

/** Whether any filter is set. */
export function hasFilters(filters: PostingFilters): boolean {
	return (
		filters.company.trim() !== '' ||
		filters.location.trim() !== '' ||
		filters.source !== '' ||
		filters.remote !== '' ||
		filters.includeRemoved
	);
}

export interface PostingsPage {
	limit?: number;
	/** Only postings with an id below this one: the next page. */
	before?: number;
}

export interface PostingsQuery {
	company?: string;
	location?: string;
	source?: Source;
	remote?: boolean;
	include_removed?: boolean;
	limit: number;
	before?: number;
}

/** The `GET /postings` query for `filters`: only the set filters, text trimmed. */
export function postingsQuery(
	filters: PostingFilters,
	{ limit = PAGE_SIZE, before }: PostingsPage = {}
): PostingsQuery {
	const query: PostingsQuery = { limit };
	const company = filters.company.trim();
	if (company) query.company = company;
	const location = filters.location.trim();
	if (location) query.location = location;
	if (filters.source) query.source = filters.source;
	if (filters.remote) query.remote = filters.remote === 'remote';
	if (filters.includeRemoved) query.include_removed = true;
	if (before !== undefined) query.before = before;
	return query;
}

/** Postings matching `filters`, newest first, a page at a time. */
export function listPostings(
	client: ApiClient,
	filters: PostingFilters,
	page: PostingsPage = {}
): Promise<Result<JobPosting[]>> {
	return run(200, () => client.GET('/postings', { params: { query: postingsQuery(filters, page) } }));
}

/** One posting with its description; a 404 is `not-found`. */
export function getPosting(client: ApiClient, id: number): Promise<Result<JobPostingDetail>> {
	return run(200, () => client.GET('/postings/{id}', { params: { path: { id } } }));
}

/** Queues a fetch of one board; a 422 is `invalid` with the field errors. */
export function sourceBoard(
	client: ApiClient,
	body: { source: Source; board: string }
): Promise<Result<SourceBoardQueued>> {
	return run(202, () => client.POST('/sourcing/boards', { body }));
}
