import { afterEach, describe, expect, it, vi } from 'vitest';
import { createApiClient, type Application, type ApplicationStatus, type Event } from '#lib/api';
import { toast } from '#lib/components/ui';
import type { Result } from './api';
import { ApplicationsStore } from './applications.svelte';

function row(id: number, overrides: Partial<Application> = {}): Application {
	return {
		id,
		company: `Company ${id}`,
		title: 'Engineer',
		url: null,
		location: null,
		comp: null,
		source: null,
		notes: null,
		resume_version_id: null,
		status: 'sourced',
		status_changed_at: '2026-10-22T00:00:00Z',
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		...overrides
	};
}

/** A promise the test settles by hand. */
function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

type ListResult = Result<Application[]>;

function setup(first: ListResult | Promise<ListResult> = { kind: 'ok', data: [] }) {
	const list = vi.fn<(client: unknown) => Promise<ListResult>>(async () => first);
	const setStatus =
		vi.fn<(client: unknown, id: number, status: ApplicationStatus) => Promise<Result<Application>>>();
	const store = new ApplicationsStore({
		client: createApiClient({ baseUrl: 'http://api.test', fetch: vi.fn() }),
		list,
		setStatus
	});
	return { store, list, setStatus };
}

/** Each held application as `[id, status, title]`, in list order. */
function rows(store: ApplicationsStore) {
	return store.applications.map((a) => [a.id, a.status, a.title]);
}

function toasts() {
	return toast.items.map((t) => [t.tone, t.title]);
}

async function settle() {
	for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => toast.clear());

const a = row(1, { updated_at: '2026-10-22T00:00:01Z' });
const b = row(2, { status: 'applied', updated_at: '2026-10-22T00:00:02Z' });

describe('ApplicationsStore', () => {
	it('loads, sorts and groups into board columns', async () => {
		const tie = row(3, { status: 'applied', updated_at: '2026-10-22T00:00:02Z' });
		const { store } = setup({ kind: 'ok', data: [a, b, tie] });
		expect(store.state).toEqual({ kind: 'loading' });
		await store.load();
		expect(store.state).toEqual({ kind: 'loaded' });
		expect(store.applications.map((x) => x.id)).toEqual([3, 2, 1]);
		expect(store.columns.map((c) => [c.status, c.applications.map((x) => x.id)])).toEqual([
			['sourced', [1]],
			['applied', [3, 2]],
			['interviewing', []],
			['offer', []],
			['rejected', []]
		]);

		const failing = setup({ kind: 'failed' });
		await failing.store.load();
		expect(failing.store.state).toEqual({ kind: 'error' });
	});

	const cases: { name: string; event: Event; want: (string | number)[][] }[] = [
		{
			name: 'created inserts a new application',
			event: {
				type: 'application.created',
				application: row(5, { updated_at: '2026-10-22T00:00:03Z' })
			},
			want: [
				[5, 'sourced', 'Engineer'],
				[2, 'applied', 'Engineer'],
				[1, 'sourced', 'Engineer']
			]
		},
		{
			name: 'a newer update replaces and re-sorts',
			event: {
				type: 'application.updated',
				application: { ...a, status: 'offer', updated_at: '2026-10-22T00:00:09Z' }
			},
			want: [
				[1, 'offer', 'Engineer'],
				[2, 'applied', 'Engineer']
			]
		},
		{
			name: 'an older update is ignored',
			event: {
				type: 'application.updated',
				application: { ...b, title: 'Stale', updated_at: '2026-10-22T00:00:00Z' }
			},
			want: [
				[2, 'applied', 'Engineer'],
				[1, 'sourced', 'Engineer']
			]
		},
		{
			name: 'on an updated_at tie the event wins',
			event: { type: 'application.updated', application: { ...b, title: 'Renamed' } },
			want: [
				[2, 'applied', 'Renamed'],
				[1, 'sourced', 'Engineer']
			]
		},
		{
			name: 'an update for an unknown id inserts it',
			event: {
				type: 'application.updated',
				application: row(9, { status: 'rejected', updated_at: '2026-10-22T00:00:00Z' })
			},
			want: [
				[2, 'applied', 'Engineer'],
				[1, 'sourced', 'Engineer'],
				[9, 'rejected', 'Engineer']
			]
		},
		{
			name: 'deleted removes it',
			event: { type: 'application.deleted', id: 2 },
			want: [[1, 'sourced', 'Engineer']]
		},
		{
			name: 'deleting an unknown id changes nothing',
			event: { type: 'application.deleted', id: 42 },
			want: [
				[2, 'applied', 'Engineer'],
				[1, 'sourced', 'Engineer']
			]
		},
		{
			name: 'other events change nothing',
			event: { type: 'queue.resumed' },
			want: [
				[2, 'applied', 'Engineer'],
				[1, 'sourced', 'Engineer']
			]
		}
	];

	it('applies events', async () => {
		for (const { name, event, want } of cases) {
			const { store } = setup({ kind: 'ok', data: [a, b] });
			await store.load();
			store.apply(event);
			expect(rows(store), name).toEqual(want);
		}

		// Events before the first load finishes are dropped: that load reads the current state.
		const pending = deferred<ListResult>();
		const { store } = setup(pending.promise);
		const loading = store.load();
		store.apply({ type: 'application.created', application: row(5) });
		store.apply({ type: 'application.deleted', id: 1 });
		pending.resolve({ kind: 'ok', data: [a] });
		await loading;
		expect(rows(store)).toEqual([[1, 'sourced', 'Engineer']]);
	});

	it('refetches silently on reconnect and lag', async () => {
		const { store, list } = setup({ kind: 'ok', data: [a] });
		// Before the first load, a reconnect doesn't fetch.
		store.apply({ type: 'app.connected' });
		expect(list).not.toHaveBeenCalled();
		await store.load();
		expect(list).toHaveBeenCalledOnce();

		const refetch = deferred<ListResult>();
		list.mockImplementation(() => refetch.promise);
		store.apply({ type: 'app.connected' });
		expect(store.state).toEqual({ kind: 'loaded' });
		// No second refetch while one is running.
		store.apply({ type: 'app.lagged', skipped: 3 });
		expect(list).toHaveBeenCalledTimes(2);
		refetch.resolve({ kind: 'ok', data: [a, b] });
		await settle();
		expect(rows(store)).toEqual([
			[2, 'applied', 'Engineer'],
			[1, 'sourced', 'Engineer']
		]);

		// A failed refetch keeps the data on screen.
		list.mockImplementation(async () => ({ kind: 'failed' }));
		store.apply({ type: 'app.lagged', skipped: 3 });
		await settle();
		expect(list).toHaveBeenCalledTimes(3);
		expect(store.state).toEqual({ kind: 'loaded' });
		expect(rows(store)).toHaveLength(2);
	});

	it('move succeeds', async () => {
		const { store, setStatus } = setup({ kind: 'ok', data: [a, b] });
		await store.load();
		const stored = { ...a, status: 'interviewing' as const, updated_at: '2026-10-22T00:00:05Z' };
		const response = deferred<Result<Application>>();
		setStatus.mockImplementation(() => response.promise);

		const moving = store.move(1, 'interviewing');
		// Optimistic: at the top of its new column at once, and pending.
		expect(store.columns[2].applications.map((x) => x.id)).toEqual([1]);
		expect(store.columns[0].applications).toEqual([]);
		expect(store.applications[0].id).toBe(1);
		expect(store.pending.has(1)).toBe(true);
		expect(setStatus).toHaveBeenCalledOnce();
		expect(setStatus.mock.calls[0].slice(1)).toEqual([1, 'interviewing']);

		response.resolve({ kind: 'ok', data: stored });
		expect(await moving).toBe(true);
		expect(store.pending.has(1)).toBe(false);
		expect(store.applications.find((x) => x.id === 1)).toBe(stored);
		expect(toasts()).toEqual([['success', 'Moved to Interviewing']]);
	});

	it('move failure reverts', async () => {
		const { store, setStatus } = setup({ kind: 'ok', data: [a, b] });
		await store.load();
		setStatus.mockResolvedValue({ kind: 'failed' });

		expect(await store.move(1, 'offer')).toBe(false);
		expect(store.pending.size).toBe(0);
		expect(store.applications.find((x) => x.id === 1)).toBe(a);
		expect(rows(store)).toEqual([
			[2, 'applied', 'Engineer'],
			[1, 'sourced', 'Engineer']
		]);
		expect(toasts()).toEqual([['danger', "Couldn't move Company 1 · Engineer"]]);

		setStatus.mockResolvedValue({ kind: 'not-found' });
		expect(await store.move(1, 'offer')).toBe(false);
		expect(store.applications.find((x) => x.id === 1)).toBe(a);
	});

	it('move failure keeps a newer event row', async () => {
		const { store, setStatus } = setup({ kind: 'ok', data: [a, b] });
		await store.load();
		const response = deferred<Result<Application>>();
		setStatus.mockImplementation(() => response.promise);

		const moving = store.move(1, 'offer');
		const newer = { ...a, status: 'rejected' as const, updated_at: '2099-01-01T00:00:00Z' };
		store.apply({ type: 'application.updated', application: newer });
		response.resolve({ kind: 'failed' });
		expect(await moving).toBe(false);
		expect(store.applications.find((x) => x.id === 1)).toBe(newer);
		expect(toasts()).toEqual([['danger', "Couldn't move Company 1 · Engineer"]]);
	});

	it('move to the same status or while pending is a no-op', async () => {
		const { store, setStatus } = setup({ kind: 'ok', data: [a, b] });
		await store.load();
		expect(await store.move(2, 'applied')).toBe(false);
		expect(await store.move(99, 'applied')).toBe(false);
		expect(setStatus).not.toHaveBeenCalled();

		const response = deferred<Result<Application>>();
		setStatus.mockImplementation(() => response.promise);
		const moving = store.move(1, 'offer');
		expect(await store.move(1, 'rejected')).toBe(false);
		expect(setStatus).toHaveBeenCalledOnce();
		response.resolve({ kind: 'ok', data: { ...a, status: 'offer' } });
		await moving;
		expect(toast.items).toHaveLength(1);
	});
});
