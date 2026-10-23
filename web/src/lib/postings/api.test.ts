import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type JobPosting, type Job } from '#lib/api';
import {
	EMPTY_FILTERS,
	getPosting,
	hasFilters,
	listPostings,
	PAGE_SIZE,
	postingsQuery,
	sourceBoard,
	type PostingFilters
} from './api';

const posting: JobPosting = {
	id: 9,
	source: 'greenhouse',
	board: 'example-labs',
	external_id: '123',
	title: 'Engineer',
	company: 'Example Labs',
	url: 'https://boards.greenhouse.io/example-labs/jobs/123',
	apply_url: null,
	department: null,
	employment_type: null,
	location_raw: null,
	city: null,
	region: null,
	country: null,
	remote: false,
	posted_at: null,
	first_seen_at: '2026-10-22T00:00:00Z',
	last_seen_at: '2026-10-22T00:00:00Z',
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:00Z',
	removed_at: null
};

const job: Job = {
	id: 4,
	kind: 'source_board',
	status: 'queued',
	attempts: 0,
	max_attempts: 3,
	run_at: '2026-10-22T00:00:00Z',
	last_error: null,
	result: null,
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:00Z',
	finished_at: null
};

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function setup(response: () => Response) {
	const fetch = vi.fn<typeof globalThis.fetch>(async () => response());
	const client = createApiClient({ baseUrl: 'http://api.test', fetch });
	return { client, fetch };
}

function sent(fetch: ReturnType<typeof setup>['fetch']): Request {
	expect(fetch).toHaveBeenCalledOnce();
	const request = fetch.mock.calls[0][0];
	if (!(request instanceof Request)) throw new Error('expected a Request');
	return request;
}

const filters = (overrides: Partial<PostingFilters>): PostingFilters => ({
	...EMPTY_FILTERS,
	...overrides
});

describe('postingsQuery', () => {
	it.each([
		['no filters', EMPTY_FILTERS, {}, { limit: PAGE_SIZE }],
		['blank text', filters({ company: '   ', location: '' }), {}, { limit: PAGE_SIZE }],
		[
			'trimmed text',
			filters({ company: ' Acme ', location: ' berlin' }),
			{},
			{ company: 'Acme', location: 'berlin', limit: PAGE_SIZE }
		],
		['a source', filters({ source: 'lever' }), {}, { source: 'lever', limit: PAGE_SIZE }],
		['remote only', filters({ remote: 'remote' }), {}, { remote: true, limit: PAGE_SIZE }],
		['not remote', filters({ remote: 'onsite' }), {}, { remote: false, limit: PAGE_SIZE }],
		[
			'show removed',
			filters({ includeRemoved: true }),
			{},
			{ include_removed: true, limit: PAGE_SIZE }
		],
		['a next page', EMPTY_FILTERS, { limit: 10, before: 42 }, { limit: 10, before: 42 }]
	] as const)('%s', (_, given, page, expected) => {
		expect(postingsQuery(given, page)).toEqual(expected);
	});
});

describe('hasFilters', () => {
	it.each([
		[EMPTY_FILTERS, false],
		[filters({ company: '  ' }), false],
		[filters({ company: 'a' }), true],
		[filters({ location: 'b' }), true],
		[filters({ source: 'ashby' }), true],
		[filters({ remote: 'onsite' }), true],
		[filters({ includeRemoved: true }), true]
	])('%o is %s', (given, expected) => {
		expect(hasFilters(given)).toBe(expected);
	});
});

describe('postings api', () => {
	it('lists with only the set filters in the query', async () => {
		const { client, fetch } = setup(() => json([posting]));
		const result = await listPostings(
			client,
			filters({ company: ' Acme ', remote: 'onsite', source: 'greenhouse' }),
			{ before: 12 }
		);
		expect(result).toEqual({ kind: 'ok', data: [posting] });
		const url = new URL(sent(fetch).url);
		expect(url.pathname).toBe('/postings');
		expect(Object.fromEntries(url.searchParams)).toEqual({
			company: 'Acme',
			source: 'greenhouse',
			remote: 'false',
			limit: String(PAGE_SIZE),
			before: '12'
		});
	});

	it('maps a list 422 to invalid', async () => {
		const errors = [{ field: 'source', message: 'Unknown source' }];
		const { client } = setup(() => json({ errors }, 422));
		expect(await listPostings(client, EMPTY_FILTERS)).toEqual({ kind: 'invalid', errors });
	});

	it('gets one posting, and maps 404 to not-found', async () => {
		const detail = { ...posting, description: 'About' };
		const ok = setup(() => json(detail));
		expect(await getPosting(ok.client, 9)).toEqual({ kind: 'ok', data: detail });
		expect(sent(ok.fetch).url).toBe('http://api.test/postings/9');
		const missing = setup(() => json(null, 404));
		expect(await getPosting(missing.client, 9)).toEqual({ kind: 'not-found' });
	});

	it('posts a board fetch and reads the 202', async () => {
		const { client, fetch } = setup(() => json({ job, reused: true }, 202));
		const result = await sourceBoard(client, { source: 'lever', board: 'example-labs' });
		expect(result).toEqual({ kind: 'ok', data: { job, reused: true } });
		const request = sent(fetch);
		expect(request.method).toBe('POST');
		expect(request.url).toBe('http://api.test/sourcing/boards');
		expect(await request.json()).toEqual({ source: 'lever', board: 'example-labs' });
	});

	it('maps a board fetch 422 to invalid and a network error to failed', async () => {
		const errors = [{ field: 'board', message: 'Required' }];
		const invalid = setup(() => json({ errors }, 422));
		expect(await sourceBoard(invalid.client, { source: 'lever', board: '' })).toEqual({
			kind: 'invalid',
			errors
		});
		const down = setup(() => {
			throw new TypeError('network');
		});
		expect(await sourceBoard(down.client, { source: 'lever', board: 'x' })).toEqual({
			kind: 'failed'
		});
	});
});
