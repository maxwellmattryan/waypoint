import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApiClient, type Memory } from '#lib/api';
import { toast } from '#lib/components/ui';
import type { MemoryFilters, Result } from './api';
import { MemoryList, SEARCH_DEBOUNCE_MS, matches } from './memories.svelte';

function memory(id: number, overrides: Partial<Memory> = {}): Memory {
	return {
		id,
		kind: 'fact',
		topic: 'skill',
		content: `Memory ${id}`,
		tags: [],
		status: 'confirmed',
		confirmed_at: '2026-10-22T00:00:00Z',
		source_kind: 'manual',
		source_ref: null,
		source_label: null,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		...overrides
	};
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

const client = createApiClient({ baseUrl: 'http://api.test', fetch: vi.fn() });
const ok = <T>(data: T): Result<T> => ({ kind: 'ok', data });
const toasts = () => toast.items.map((t) => [t.tone, t.title]);
const ids = (list: MemoryList) => list.memories.map((m) => m.id);

function setup(memories: Memory[] = [memory(1), memory(2)]) {
	const list = vi.fn<(client: unknown, filters?: MemoryFilters) => Promise<Result<Memory[]>>>(
		async () => ok(memories)
	);
	const confirm = vi.fn(async (_client: unknown, id: number) =>
		ok(memory(id, { status: 'confirmed' }))
	);
	const remove = vi.fn(async (): Promise<Result<void>> => ok(undefined));
	const store = new MemoryList({ client, list, confirm, remove });
	return { store, list, confirm, remove };
}

beforeEach(() => {
	for (const item of [...toast.items]) toast.dismiss(item.id);
});

afterEach(() => {
	vi.useRealTimers();
});

describe('MemoryList', () => {
	it('loads with the current filters and exposes loading, loaded and error', async () => {
		const { store, list } = setup();
		expect(store.view.kind).toBe('loading');
		await store.load();
		expect(store.view.kind).toBe('loaded');
		expect(ids(store)).toEqual([1, 2]);
		expect(list).toHaveBeenLastCalledWith(client, { q: '', status: '', tag: '' });

		list.mockResolvedValueOnce({ kind: 'failed' });
		await store.load();
		expect(store.view.kind).toBe('error');
		list.mockResolvedValueOnce(ok([memory(3)]));
		const reload = store.load();
		expect(store.view.kind).toBe('loading');
		await reload;
		expect(ids(store)).toEqual([3]);
	});

	it('reloads at once when the status or tag filter changes, and on clear', async () => {
		const { store, list } = setup();
		await store.load();
		await store.setStatus('proposed');
		expect(list).toHaveBeenLastCalledWith(client, { q: '', status: 'proposed', tag: '' });
		await store.setTag('  Remote  First ');
		expect(list).toHaveBeenLastCalledWith(client, {
			q: '',
			status: 'proposed',
			tag: 'remote first'
		});
		expect(store.filtered).toBe(true);
		await store.clearFilters();
		expect(list).toHaveBeenLastCalledWith(client, { q: '', status: '', tag: '' });
		expect(store.filtered).toBe(false);
	});

	it('debounces search by 300ms after the last keystroke', async () => {
		vi.useFakeTimers();
		const { store, list } = setup();
		await store.load();
		list.mockClear();
		store.setQuery('r');
		await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1);
		store.setQuery('re');
		await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1);
		store.setQuery('remote');
		await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1);
		expect(list).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);
		expect(list).toHaveBeenCalledTimes(1);
		expect(list).toHaveBeenLastCalledWith(client, { q: 'remote', status: '', tag: '' });
	});

	it('ignores a response for an older filter state', async () => {
		const { store, list } = setup();
		await store.load();
		const slow = deferred<Result<Memory[]>>();
		list.mockReturnValueOnce(slow.promise);
		list.mockResolvedValueOnce(ok([memory(9)]));
		const first = store.setStatus('proposed');
		await store.setStatus('confirmed');
		slow.resolve(ok([memory(8)]));
		await first;
		expect(ids(store)).toEqual([9]);
	});

	it('inserts a created memory first and replaces an edited one in place', async () => {
		const { store, list } = setup();
		await store.load();
		list.mockClear();
		store.saved(memory(3), true);
		expect(ids(store)).toEqual([3, 1, 2]);
		store.saved(memory(2, { content: 'Edited' }), false);
		expect(ids(store)).toEqual([3, 1, 2]);
		expect(store.memories[2].content).toBe('Edited');
		expect(list).not.toHaveBeenCalled();
		expect(toasts()).toContainEqual(['success', 'Memory saved']);
	});

	it('reloads when a saved memory no longer matches the filters', async () => {
		const { store, list } = setup();
		await store.setTag('remote');
		list.mockClear();
		store.saved(memory(1, { tags: ['onsite'] }), false);
		expect(list).toHaveBeenCalledTimes(1);

		await store.setTag('');
		store.setQuery('rust');
		await store.load();
		list.mockClear();
		store.saved(memory(4), true);
		expect(list).toHaveBeenCalledTimes(1);
	});

	it('removes a deleted memory, also on 404, and keeps it on other failures', async () => {
		const { store, remove } = setup([memory(1), memory(2), memory(3)]);
		await store.load();
		expect(await store.delete(2)).toBe(true);
		expect(ids(store)).toEqual([1, 3]);
		expect(toasts()).toContainEqual(['success', 'Memory deleted']);

		remove.mockResolvedValueOnce({ kind: 'not-found' });
		expect(await store.delete(3)).toBe(true);
		expect(ids(store)).toEqual([1]);

		remove.mockResolvedValueOnce({ kind: 'failed' });
		expect(await store.delete(1)).toBe(false);
		expect(ids(store)).toEqual([1]);
		expect(toasts()).toContainEqual(['danger', "Couldn't delete the memory"]);
	});

	it('updates a confirmed memory in place, and toasts a failure', async () => {
		const { store, confirm } = setup([memory(1, { status: 'proposed', confirmed_at: null })]);
		await store.load();
		expect(await store.confirm(1)).toBe(true);
		expect(store.memories[0].status).toBe('confirmed');
		expect(toasts()).toContainEqual(['success', 'Memory confirmed']);

		confirm.mockResolvedValueOnce({ kind: 'failed' });
		expect(await store.confirm(1)).toBe(false);
		expect(toasts()).toContainEqual(['danger', "Couldn't confirm the memory"]);
	});

	it('reloads after confirming under a Proposed filter', async () => {
		const { store, list } = setup([memory(1, { status: 'proposed', confirmed_at: null })]);
		await store.setStatus('proposed');
		list.mockClear();
		list.mockResolvedValueOnce(ok([]));
		await store.confirm(1);
		await vi.waitFor(() => expect(list).toHaveBeenCalledTimes(1));
		await vi.waitFor(() => expect(ids(store)).toEqual([]));
	});
});

describe('matches', () => {
	const filters = { q: '', status: '' as const, tag: '' };

	it('holds without filters and checks status and tag', () => {
		expect(matches(memory(1), filters)).toBe(true);
		expect(matches(memory(1), { ...filters, status: 'proposed' })).toBe(false);
		expect(matches(memory(1, { tags: ['remote'] }), { ...filters, tag: ' Remote ' })).toBe(true);
		expect(matches(memory(1), { ...filters, tag: 'remote' })).toBe(false);
	});

	it('never holds while searching, since the server ranks', () => {
		expect(matches(memory(1), { ...filters, q: 'skill' })).toBe(false);
	});
});
