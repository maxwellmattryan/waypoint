import { afterEach, describe, expect, it, vi } from 'vitest';
import { createApiClient, type Event, type Run, type RunDetail } from '#lib/api';
import { toast } from '#lib/components/ui';
import { PAGE_SIZE, type ListRunsOptions, type Result } from './api';
import { RunDetailStore, RunsStore } from './runs.svelte';

function makeRun(id: number, overrides: Partial<Run> = {}): Run {
	return {
		id,
		workflow: 'triage',
		status: 'queued',
		input: {},
		output: null,
		error: null,
		input_tokens: 0,
		output_tokens: 0,
		cost_usd: 0,
		job_id: id,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:01Z',
		started_at: null,
		finished_at: null,
		...overrides
	};
}

function detail(id: number, overrides: Partial<Run> = {}): RunDetail {
	return { ...makeRun(id, overrides), steps: [] };
}

/** `count` runs counting down from `top`. */
function page(top: number, count: number): Run[] {
	return Array.from({ length: count }, (_, i) => makeRun(top - i));
}

/** A promise the test settles by hand. */
function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

async function settle() {
	for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

const client = createApiClient({ baseUrl: 'http://api.test', fetch: vi.fn() });
const updated = (run: Run): Event => ({ type: 'run.updated', run });

afterEach(() => toast.clear());

type ListResult = Result<Run[]>;

function setupList(...results: ListResult[]) {
	const list = vi.fn<(client: unknown, options?: ListRunsOptions) => Promise<ListResult>>();
	for (const result of results) list.mockResolvedValueOnce(result);
	const store = new RunsStore({ client, list });
	return { store, list };
}

const ids = (store: RunsStore) => store.runs.map((run) => run.id);

describe('RunsStore', () => {
	it('loads the first page', async () => {
		const { store, list } = setupList({ kind: 'ok', data: page(3, 3) });
		expect(store.state).toEqual({ kind: 'loading' });
		await store.load();
		expect(store.state).toEqual({ kind: 'loaded' });
		expect(ids(store)).toEqual([3, 2, 1]);
		expect(store.hasMore).toBe(false);
		expect(list).toHaveBeenCalledWith(client, { limit: PAGE_SIZE });
	});

	it('reports a failed first load and recovers on retry', async () => {
		const { store } = setupList({ kind: 'failed' }, { kind: 'ok', data: [] });
		await store.load();
		expect(store.state).toEqual({ kind: 'error' });
		await store.load();
		expect(store.state).toEqual({ kind: 'loaded' });
		expect(store.runs).toEqual([]);
	});

	it('pages with before until a short page', async () => {
		const { store, list } = setupList(
			{ kind: 'ok', data: page(120, PAGE_SIZE) },
			{ kind: 'ok', data: page(70, PAGE_SIZE) },
			{ kind: 'ok', data: page(20, 3) }
		);
		await store.load();
		expect(store.hasMore).toBe(true);
		await store.loadMore();
		expect(list).toHaveBeenLastCalledWith(client, {
			limit: PAGE_SIZE,
			before: 71
		});
		expect(store.runs).toHaveLength(2 * PAGE_SIZE);
		expect(store.hasMore).toBe(true);
		await store.loadMore();
		expect(list).toHaveBeenLastCalledWith(client, {
			limit: PAGE_SIZE,
			before: 21
		});
		expect(ids(store).slice(-3)).toEqual([20, 19, 18]);
		expect(store.hasMore).toBe(false);
		await store.loadMore();
		expect(list).toHaveBeenCalledTimes(3);
	});

	it('keeps its runs when a page fails', async () => {
		const { store } = setupList({ kind: 'ok', data: page(60, PAGE_SIZE) }, { kind: 'failed' });
		await store.load();
		await store.loadMore();
		expect(store.runs).toHaveLength(PAGE_SIZE);
		expect(store.hasMore).toBe(true);
		expect(store.loadingMore).toBe(false);
		expect(toast.items.map((t) => [t.tone, t.title])).toEqual([
			['danger', "Couldn't load more runs"]
		]);
	});

	it('upserts on run.updated: newer wins, new runs go on top, older than shown are ignored', async () => {
		const { store } = setupList({
			kind: 'ok',
			data: [makeRun(5), makeRun(4), makeRun(3)]
		});
		await store.load();

		store.handle(updated(makeRun(4, { status: 'running', updated_at: '2026-10-22T00:00:05Z' })));
		expect(store.runs.find((run) => run.id === 4)?.status).toBe('running');

		store.handle(updated(makeRun(4, { status: 'queued', updated_at: '2026-10-22T00:00:02Z' })));
		expect(store.runs.find((run) => run.id === 4)?.status).toBe('running');

		store.handle(updated(makeRun(6)));
		expect(ids(store)).toEqual([6, 5, 4, 3]);

		store.handle(updated(makeRun(2)));
		expect(ids(store)).toEqual([6, 5, 4, 3]);
	});

	it('drops events before the first load', async () => {
		const { store } = setupList({ kind: 'ok', data: [makeRun(1)] });
		store.handle(updated(makeRun(9)));
		await store.load();
		expect(ids(store)).toEqual([1]);
	});

	it.each(['app.connected', 'app.lagged'] as const)(
		'refetches silently on %s and keeps older pages',
		async (type) => {
			const { store, list } = setupList(
				{ kind: 'ok', data: page(60, PAGE_SIZE) },
				{ kind: 'ok', data: page(10, 3) },
				{
					kind: 'ok',
					data: [
						makeRun(61),
						makeRun(60, {
							status: 'succeeded',
							updated_at: '2026-10-22T00:01:00Z'
						}),
						...page(59, PAGE_SIZE - 2)
					]
				}
			);
			await store.load();
			await store.loadMore();
			const event: Event = type === 'app.lagged' ? { type, skipped: 3 } : { type };
			store.handle(event);
			expect(store.state).toEqual({ kind: 'loaded' });
			await settle();
			expect(list).toHaveBeenCalledTimes(3);
			expect(list).toHaveBeenLastCalledWith(client, { limit: PAGE_SIZE });
			expect(ids(store)[0]).toBe(61);
			expect(store.runs.find((run) => run.id === 60)?.status).toBe('succeeded');
			expect(ids(store).slice(-3)).toEqual([10, 9, 8]);
		}
	);

	it('drops every page and refetches on data.deleted', async () => {
		const { store, list } = setupList(
			{ kind: 'ok', data: page(60, PAGE_SIZE) },
			{ kind: 'ok', data: page(10, 3) },
			{ kind: 'ok', data: [] }
		);
		await store.load();
		await store.loadMore();
		store.handle({ type: 'data.deleted' });
		await settle();
		expect(list).toHaveBeenCalledTimes(3);
		expect(list).toHaveBeenLastCalledWith(client, { limit: PAGE_SIZE });
		expect(ids(store)).toEqual([]);
		expect(store.hasMore).toBe(false);
		expect(store.state).toEqual({ kind: 'loaded' });
	});
});

type GetResult = Result<RunDetail>;

function setupDetail(id: number, ...results: (GetResult | Promise<GetResult>)[]) {
	const get = vi.fn<(client: unknown, id: number) => Promise<GetResult>>();
	for (const result of results) get.mockImplementationOnce(async () => result);
	const store = new RunDetailStore(id, { client, get });
	return { store, get };
}

describe('RunDetailStore', () => {
	it('loads a run', async () => {
		const { store, get } = setupDetail(1, { kind: 'ok', data: detail(1) });
		expect(store.state).toEqual({ kind: 'loading' });
		await store.load();
		expect(store.run?.id).toBe(1);
		expect(get).toHaveBeenCalledWith(client, 1);
	});

	it.each([
		[{ kind: 'not-found' } as GetResult, 'not-found'],
		[{ kind: 'failed' } as GetResult, 'error']
	])('maps %o to %s', async (result, kind) => {
		const { store } = setupDetail(1, result);
		await store.load();
		expect(store.state).toEqual({ kind });
		expect(store.run).toBeNull();
	});

	it('refetches only for its own run id', async () => {
		const { store, get } = setupDetail(
			1,
			{ kind: 'ok', data: detail(1) },
			{ kind: 'ok', data: detail(1, { status: 'running' }) }
		);
		await store.load();
		store.handle(updated(makeRun(2)));
		store.handle({ type: 'application.deleted', id: 1 });
		await settle();
		expect(get).toHaveBeenCalledTimes(1);
		store.handle(updated(makeRun(1)));
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		expect(store.run?.status).toBe('running');
	});

	it('coalesces a burst into one request in flight and one queued', async () => {
		const first = deferred<GetResult>();
		const second = deferred<GetResult>();
		const { store, get } = setupDetail(
			1,
			{ kind: 'ok', data: detail(1) },
			first.promise,
			second.promise
		);
		await store.load();
		for (let i = 0; i < 5; i++) store.handle(updated(makeRun(1)));
		await settle();
		expect(get).toHaveBeenCalledTimes(2);

		first.resolve({ kind: 'ok', data: detail(1, { status: 'running' }) });
		await settle();
		expect(get).toHaveBeenCalledTimes(3);
		expect(store.run?.status).toBe('running');

		second.resolve({ kind: 'ok', data: detail(1, { status: 'succeeded' }) });
		await settle();
		expect(get).toHaveBeenCalledTimes(3);
		expect(store.run?.status).toBe('succeeded');
	});

	it('refetches once on reconnect and keeps the run when that fails', async () => {
		const { store, get } = setupDetail(1, { kind: 'ok', data: detail(1) }, { kind: 'failed' });
		await store.load();
		store.handle({ type: 'app.connected' });
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		expect(store.state.kind).toBe('loaded');
		expect(store.run?.id).toBe(1);
	});

	it('shows not found once data.deleted removed the run', async () => {
		const { store, get } = setupDetail(1, { kind: 'ok', data: detail(1) }, { kind: 'not-found' });
		await store.load();
		store.handle({ type: 'data.deleted' });
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		expect(store.state).toEqual({ kind: 'not-found' });
	});
});
