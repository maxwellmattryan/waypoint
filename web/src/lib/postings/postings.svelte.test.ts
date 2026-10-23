import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	createApiClient,
	type Event,
	type Job,
	type JobPosting,
	type JobPostingDetail,
	type PostingFit,
	type Run,
	type ScoreFitQueued
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { EMPTY_FILTERS, PAGE_SIZE, type PostingFilters, type PostingsPage, type Result } from './api';
import {
	FILTER_DEBOUNCE_MS,
	PostingDetailStore,
	PostingFitStore,
	PostingsStore
} from './postings.svelte';

function makePosting(id: number, overrides: Partial<JobPosting> = {}): JobPosting {
	return {
		id,
		source: 'greenhouse',
		board: 'example-labs',
		external_id: String(id),
		title: `Role ${id}`,
		company: 'Example Labs',
		url: `https://boards.greenhouse.io/example-labs/jobs/${id}`,
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
		removed_at: null,
		fit_score: null,
		fit_band: null,
		fit_in_review: false,
		...overrides
	};
}

/** `count` postings counting down from `top`. */
function page(top: number, count: number): JobPosting[] {
	return Array.from({ length: count }, (_, i) => makePosting(top - i));
}

function makeJob(id: number, overrides: Partial<Job> = {}): Job {
	return {
		id,
		kind: 'source_board',
		status: 'succeeded',
		attempts: 1,
		max_attempts: 3,
		run_at: '2026-10-22T00:00:00Z',
		last_error: null,
		result: null,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		finished_at: '2026-10-22T00:00:01Z',
		...overrides
	};
}

/** A promise the test settles by hand. */
function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

async function settle() {
	for (let i = 0; i < 5; i++) await Promise.resolve();
}

const client = createApiClient({ baseUrl: 'http://api.test', fetch: vi.fn() });

afterEach(() => {
	toast.clear();
	vi.useRealTimers();
});

type ListResult = Result<JobPosting[]>;

function setup(...results: (ListResult | Promise<ListResult>)[]) {
	const list =
		vi.fn<(client: unknown, filters: PostingFilters, page?: PostingsPage) => Promise<ListResult>>();
	for (const result of results) list.mockReturnValueOnce(Promise.resolve(result));
	list.mockResolvedValue({ kind: 'ok', data: [] });
	const store = new PostingsStore({ client, list });
	return { store, list };
}

const ids = (store: PostingsStore) => store.postings.map((posting) => posting.id);
const ok = (data: JobPosting[]): ListResult => ({ kind: 'ok', data });
const toasts = () => toast.items.map((t) => [t.tone, t.title]);

describe('PostingsStore', () => {
	it('loads the first page with no filters, then recovers from a failed load on retry', async () => {
		const { store, list } = setup({ kind: 'failed' }, ok(page(3, 3)));
		expect(store.view).toEqual({ kind: 'loading' });
		await store.load();
		expect(list).toHaveBeenLastCalledWith(client, EMPTY_FILTERS, { limit: PAGE_SIZE });
		expect(store.view).toEqual({ kind: 'error' });
		await store.load();
		expect(store.view).toEqual({ kind: 'loaded' });
		expect(ids(store)).toEqual([3, 2, 1]);
		expect(store.hasMore).toBe(false);
	});

	it('reloads at once on a select or checkbox change, keeping the list shown meanwhile', async () => {
		const reload = deferred<ListResult>();
		const { store, list } = setup(ok(page(3, 3)), reload.promise);
		await store.load();
		const loading = store.setSource('lever');
		expect(list).toHaveBeenLastCalledWith(client, { ...EMPTY_FILTERS, source: 'lever' }, {
			limit: PAGE_SIZE
		});
		expect(store.view).toEqual({ kind: 'loaded' });
		expect(ids(store)).toEqual([3, 2, 1]);
		reload.resolve(ok([makePosting(2)]));
		await loading;
		expect(ids(store)).toEqual([2]);
		expect(store.filtered).toBe(true);

		await store.setRemote('onsite');
		await store.setIncludeRemoved(true);
		expect(list).toHaveBeenLastCalledWith(
			client,
			{ ...EMPTY_FILTERS, source: 'lever', remote: 'onsite', includeRemoved: true },
			{ limit: PAGE_SIZE }
		);
	});

	it('debounces the text filters by 300 ms', async () => {
		vi.useFakeTimers();
		const { store, list } = setup(ok([]));
		await store.load();
		store.setCompany('Ac');
		store.setCompany('Acme');
		store.setLocation('Berlin');
		await vi.advanceTimersByTimeAsync(FILTER_DEBOUNCE_MS - 1);
		expect(list).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(1);
		expect(list).toHaveBeenCalledTimes(2);
		expect(list).toHaveBeenLastCalledWith(
			client,
			{ ...EMPTY_FILTERS, company: 'Acme', location: 'Berlin' },
			{ limit: PAGE_SIZE }
		);
	});

	it('drops a response for an older filter set', async () => {
		const older = deferred<ListResult>();
		const { store } = setup(ok([]), older.promise, ok([makePosting(7)]));
		await store.load();
		const first = store.setSource('lever');
		await store.setSource('ashby');
		expect(ids(store)).toEqual([7]);
		older.resolve(ok([makePosting(1)]));
		await first;
		expect(ids(store)).toEqual([7]);
		expect(store.filters.source).toBe('ashby');
	});

	it('shows the error state when a reload fails, and recovers on retry', async () => {
		const { store } = setup(ok(page(3, 3)), { kind: 'failed' }, ok([makePosting(1)]));
		await store.load();
		await store.setSource('ashby');
		expect(store.view).toEqual({ kind: 'error' });
		await store.load();
		expect(store.view).toEqual({ kind: 'loaded' });
		expect(ids(store)).toEqual([1]);
	});

	it('pages with before, then a filter change goes back to the first page', async () => {
		const { store, list } = setup(
			ok(page(120, PAGE_SIZE)),
			ok(page(70, 3)),
			ok([makePosting(5)])
		);
		await store.load();
		expect(store.hasMore).toBe(true);
		await store.loadMore();
		expect(list).toHaveBeenLastCalledWith(client, EMPTY_FILTERS, {
			limit: PAGE_SIZE,
			before: 71
		});
		expect(store.postings).toHaveLength(PAGE_SIZE + 3);
		expect(store.hasMore).toBe(false);
		await store.loadMore();
		expect(list).toHaveBeenCalledTimes(2);

		await store.setRemote('remote');
		expect(list).toHaveBeenLastCalledWith(
			client,
			{ ...EMPTY_FILTERS, remote: 'remote' },
			{ limit: PAGE_SIZE }
		);
		expect(ids(store)).toEqual([5]);
	});

	it('keeps the list and toasts when a page fails', async () => {
		const { store } = setup(ok(page(60, PAGE_SIZE)), { kind: 'failed' });
		await store.load();
		await store.loadMore();
		expect(store.postings).toHaveLength(PAGE_SIZE);
		expect(store.loadingMore).toBe(false);
		expect(toasts()).toEqual([['danger', "Couldn't load more postings"]]);
	});

	it('clears every filter and reloads', async () => {
		vi.useFakeTimers();
		const { store, list } = setup(ok([]));
		await store.load();
		store.setCompany('Acme');
		await store.setSource('lever');
		await store.clearFilters();
		expect(store.filters).toEqual(EMPTY_FILTERS);
		expect(store.filtered).toBe(false);
		expect(list).toHaveBeenLastCalledWith(client, EMPTY_FILTERS, { limit: PAGE_SIZE });
		await vi.advanceTimersByTimeAsync(FILTER_DEBOUNCE_MS);
		expect(list).toHaveBeenCalledTimes(3);
	});

	it('clears the list and reloads on data.deleted', async () => {
		const reload = deferred<ListResult>();
		const { store, list } = setup(ok(page(60, PAGE_SIZE)), reload.promise);
		await store.load();
		store.handle({ type: 'data.deleted' });
		expect(store.postings).toEqual([]);
		expect(store.hasMore).toBe(false);
		expect(list).toHaveBeenCalledTimes(2);
		reload.resolve(ok([]));
		await settle();
		expect(store.view).toEqual({ kind: 'loaded' });
	});

	it.each([
		{ type: 'app.connected' },
		{ type: 'app.lagged', skipped: 2 },
		{ type: 'job.succeeded', job: makeJob(99) }
	] as Event[])('replaces the list silently on $type', async (event) => {
		const { store, list } = setup(
			ok(page(120, PAGE_SIZE)),
			ok(page(70, 3)),
			ok([makePosting(121), ...page(120, 2)])
		);
		await store.load();
		await store.loadMore();
		store.handle(event);
		expect(store.view).toEqual({ kind: 'loaded' });
		await settle();
		expect(list).toHaveBeenLastCalledWith(client, EMPTY_FILTERS, { limit: PAGE_SIZE });
		expect(ids(store)).toEqual([121, 120, 119]);
		expect(toasts()).toEqual([]);
	});

	it('refreshes when a score_fit run succeeds, and not for other runs', async () => {
		const { store, list } = setup(ok([]));
		await store.load();
		store.handle(runUpdated({ workflow: 'score_fit', status: 'running' }));
		store.handle(runUpdated({ workflow: 'classify_email', status: 'succeeded' }));
		await settle();
		expect(list).toHaveBeenCalledTimes(1);
		store.handle(runUpdated({ workflow: 'score_fit', status: 'succeeded' }));
		await settle();
		expect(list).toHaveBeenCalledTimes(2);
	});

	it('ignores events before the first load, and jobs of other kinds', async () => {
		const { store, list } = setup(ok([]));
		store.handle({ type: 'app.connected' });
		store.handle({ type: 'job.succeeded', job: makeJob(1) });
		expect(list).not.toHaveBeenCalled();
		await store.load();
		store.handle({ type: 'job.succeeded', job: makeJob(2, { kind: 'capture_extract' }) });
		await settle();
		expect(list).toHaveBeenCalledTimes(1);
	});

	it('toasts the counts of a remembered job once', async () => {
		const { store } = setup(ok([]));
		await store.load();
		store.track(4, 'example-labs');
		store.track(5, 'acme');
		const done = makeJob(4, { result: { inserted: 3, updated: 1, removed: 2, fetched: 4 } });
		store.handle({ type: 'job.succeeded', job: done });
		store.handle({ type: 'job.succeeded', job: done });
		store.handle({ type: 'job.succeeded', job: makeJob(5, { result: { inserted: 1 } }) });
		store.handle({ type: 'job.succeeded', job: makeJob(6, { result: { inserted: 8 } }) });
		expect(toasts()).toEqual([
			['success', 'example-labs: 3 new, 1 updated, 2 removed'],
			['success', 'acme: 1 new, 0 updated, 0 removed']
		]);
	});

	it('toasts the failure of a remembered job once, and nothing for other jobs', async () => {
		const { store, list } = setup(ok([]));
		await store.load();
		store.track(4, 'example-labs');
		const failed = makeJob(4, { status: 'failed', last_error: 'the fetch failed' });
		store.handle({ type: 'job.failed', job: failed });
		store.handle({ type: 'job.failed', job: failed });
		store.handle({ type: 'job.failed', job: makeJob(8, { status: 'failed' }) });
		expect(toasts()).toEqual([
			['danger', "Couldn't source example-labs. Check the source and the board slug."]
		]);
		await settle();
		expect(list).toHaveBeenCalledTimes(1);
	});
});

function makeRun(overrides: Partial<Run> = {}): Run {
	return {
		id: 40,
		workflow: 'score_fit',
		status: 'queued',
		input: { posting_id: 9 },
		output: null,
		error: null,
		input_tokens: 0,
		output_tokens: 0,
		cost_usd: 0,
		job_id: 1,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		started_at: null,
		finished_at: null,
		...overrides
	};
}

function runUpdated(overrides: Partial<Run> = {}): Event {
	return { type: 'run.updated', run: makeRun(overrides) };
}

type GetResult = Result<JobPostingDetail>;

describe('PostingDetailStore', () => {
	it.each([
		[{ kind: 'ok', data: { ...makePosting(9), description: '' } }, 'loaded'],
		[{ kind: 'not-found' }, 'not-found'],
		[{ kind: 'failed' }, 'error']
	] as [GetResult, string][])('maps %o to %s', async (result, kind) => {
		const get = vi.fn(async () => result);
		const store = new PostingDetailStore(9, { client, get });
		expect(store.state.kind).toBe('loading');
		await store.load();
		expect(get).toHaveBeenCalledWith(client, 9);
		expect(store.state.kind).toBe(kind);
	});
});

describe('PostingFitStore', () => {
	const empty: PostingFit = { posting_id: 9, current: null, in_review: null, run: null };

	function setupFit(...results: Result<PostingFit>[]) {
		const get = vi.fn<(c: unknown, id: number) => Promise<Result<PostingFit>>>();
		for (const result of results) get.mockResolvedValueOnce(result);
		get.mockResolvedValue({ kind: 'ok', data: empty });
		const score = vi.fn<(c: unknown, id: number) => Promise<Result<ScoreFitQueued>>>();
		const store = new PostingFitStore(9, { client, get, score });
		return { store, get, score };
	}

	it('loads the fit, and shows the error state until a load succeeds', async () => {
		const { store, get } = setupFit({ kind: 'failed' });
		expect(store.state.kind).toBe('loading');
		await store.load();
		expect(get).toHaveBeenCalledWith(client, 9);
		expect(store.state).toEqual({ kind: 'error' });
		await store.load();
		expect(store.state).toEqual({ kind: 'loaded', fit: empty, view: { kind: 'empty' } });
	});

	it('posts a score and shows the scoring state at once', async () => {
		const { store, score } = setupFit();
		await store.load();
		score.mockResolvedValueOnce({ kind: 'ok', data: { run: makeRun(), reused: false } });
		const started = store.score();
		expect(store.starting).toBe(true);
		await started;
		expect(score).toHaveBeenCalledWith(client, 9);
		expect(store.starting).toBe(false);
		expect(store.state.kind === 'loaded' && store.state.view).toEqual({
			kind: 'scoring',
			run: { id: 40, status: 'queued', error: null }
		});
	});

	it('toasts when a score cannot start', async () => {
		const { store, score } = setupFit();
		await store.load();
		score.mockResolvedValueOnce({ kind: 'failed' });
		await store.score();
		expect(store.state.kind === 'loaded' && store.state.view).toEqual({ kind: 'empty' });
		expect(toasts()).toEqual([['danger', "Couldn't start scoring"]]);
	});

	it('reloads on run.updated for its own score_fit run only', async () => {
		const { store, get } = setupFit();
		await store.load();
		store.handle(runUpdated({ input: { posting_id: 10 } }));
		store.handle(runUpdated({ workflow: 'classify_email', input: { posting_id: 9 } }));
		store.handle(runUpdated({ input: { email_id: 9 } }));
		await settle();
		expect(get).toHaveBeenCalledTimes(1);
		store.handle(runUpdated({ status: 'succeeded' }));
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		store.handle({ type: 'app.connected' });
		store.handle({ type: 'app.lagged', skipped: 3 });
		await settle();
		expect(get).toHaveBeenCalledTimes(4);
	});

	it('drops a response older than the latest load', async () => {
		const first = deferred<Result<PostingFit>>();
		const get = vi.fn<(c: unknown, id: number) => Promise<Result<PostingFit>>>();
		get.mockReturnValueOnce(first.promise);
		const newer: PostingFit = { ...empty, run: { id: 41, status: 'running', error: null } };
		get.mockResolvedValueOnce({ kind: 'ok', data: newer });
		const store = new PostingFitStore(9, { client, get });
		const stale = store.load();
		await store.load();
		first.resolve({ kind: 'ok', data: empty });
		await stale;
		expect(store.state.kind === 'loaded' && store.state.view.kind).toBe('scoring');
	});
});
