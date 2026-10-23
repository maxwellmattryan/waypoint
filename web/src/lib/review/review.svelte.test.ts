import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	createApiClient,
	type AgentAction,
	type Event,
	type MemorySignal,
	type PendingAction,
	type ReviewItem,
	type Run
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { PAGE_SIZE } from '#lib/runs/api.js';
import type { ListActionsOptions, ListSignalsOptions, Result } from './api';
import {
	ActionLogStore,
	agentsPaused,
	QueueStore,
	queueKey,
	SignalLogStore
} from './review.svelte';

function pending(id: number, overrides: Partial<PendingAction> = {}): PendingAction {
	return {
		id,
		run_id: 10 + id,
		run_step_id: id,
		tool_call_id: id,
		tool: 'draft_follow_up',
		input: { message: 'Thanks' },
		approved_input: null,
		status: 'pending',
		output: null,
		executed_tool_call_id: null,
		created_at: '2026-10-22T00:00:00Z',
		decided_at: null,
		executed_at: null,
		...overrides
	};
}

function review(id: number, overrides: Partial<ReviewItem> = {}): ReviewItem {
	return {
		id,
		run_id: 20 + id,
		run_step_id: id,
		kind: 'tidy.note',
		confidence: 0.3,
		payload: { note: 'n' },
		status: 'pending',
		created_at: '2026-10-22T00:00:00Z',
		resolved_at: null,
		...overrides
	};
}

function action(id: number, overrides: Partial<AgentAction> = {}): AgentAction {
	return {
		id,
		run_id: 30 + id,
		run_step_id: id,
		tool_call_id: id,
		tool: 'add_note',
		undo_data: { event_id: id },
		created_at: '2026-10-22T00:00:00Z',
		undone_at: null,
		...overrides
	};
}

/** `count` actions counting down from `top`. */
function page(top: number, count: number): AgentAction[] {
	return Array.from({ length: count }, (_, i) => action(top - i));
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

async function settle() {
	for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

const client = createApiClient({ baseUrl: 'http://api.test', fetch: vi.fn() });
const runUpdated = { type: 'run.updated', run: { id: 1 } as Run } as Event;
const toasts = () => toast.items.map((t) => [t.tone, t.title, t.description]);

afterEach(() => toast.clear());

const ok = <T>(data: T): Result<T> => ({ kind: 'ok', data });

function setupQueue(
	actions: Result<PendingAction[]>[],
	items: Result<ReviewItem[]>[],
	deps: { paused?: boolean } = {}
) {
	const listPending = vi.fn<() => Promise<Result<PendingAction[]>>>();
	for (const result of actions) listPending.mockResolvedValueOnce(result);
	const listReview = vi.fn<() => Promise<Result<ReviewItem[]>>>();
	for (const result of items) listReview.mockResolvedValueOnce(result);
	const approve =
		vi.fn<(c: unknown, id: number, input?: Record<string, unknown>) => Promise<Result<PendingAction>>>();
	const reject = vi.fn<(c: unknown, id: number) => Promise<Result<PendingAction>>>();
	const dismiss = vi.fn<(c: unknown, id: number, reason?: string) => Promise<Result<ReviewItem>>>();
	const accept = vi.fn<(c: unknown, id: number) => Promise<Result<ReviewItem>>>();
	const store = new QueueStore({
		client,
		listPending,
		listReview,
		approve,
		reject,
		dismiss,
		accept,
		paused: () => deps.paused ?? false
	});
	return { store, listPending, listReview, approve, reject, dismiss, accept };
}

const sentEmail = {
	subject: 'Next steps',
	from_name: 'Stark Scheduling',
	from_address: 'calendar@stark.example',
	sent_at: null
};

/** An `email.match` review item for Stark Industries. */
function emailMatch(id: number): ReviewItem {
	return review(id, {
		kind: 'email.match',
		payload: {
			email_id: 3,
			classification_id: 7,
			class: 'rejection',
			email: sentEmail,
			application_id: 12,
			company: 'Stark Industries',
			title: 'Software Engineer',
			ops: [{ op: 'set_status', from: 'applied', to: 'rejected' }],
			reasons: ['company'],
			review_reasons: ['low_confidence'],
			candidates: []
		}
	});
}

/** An `email.new_application` review item for Globex. */
function newApplication(id: number): ReviewItem {
	return review(id, {
		kind: 'email.new_application',
		payload: {
			email_id: 4,
			classification_id: 8,
			class: 'other',
			email: sentEmail,
			application: { company: 'Globex', title: 'Data Engineer', status: 'applied' }
		}
	});
}

const keys = (store: QueueStore) => store.items.map(queueKey);

describe('QueueStore', () => {
	it('merges pending actions and review items oldest first, actions first on a tie', async () => {
		const { store } = setupQueue(
			[
				ok([
					pending(2, { created_at: '2026-10-22T00:00:05Z' }),
					pending(1, { created_at: '2026-10-22T00:00:03Z' }),
					pending(3, { created_at: '2026-10-22T00:00:03Z' })
				])
			],
			[
				ok([
					review(7, { created_at: '2026-10-22T00:00:03Z' }),
					review(4, { created_at: '2026-10-22T00:00:01Z' }),
					review(5, { created_at: '2026-10-22T00:00:03Z' })
				])
			]
		);
		expect(store.state).toEqual({ kind: 'loading' });
		await store.load();
		expect(store.state).toEqual({ kind: 'loaded' });
		expect(keys(store)).toEqual([
			'review:4',
			'action:1',
			'action:3',
			'review:5',
			'review:7',
			'action:2'
		]);
	});

	it.each([
		['pending actions', { kind: 'failed' } as const, ok([])],
		['review items', ok([]), { kind: 'failed' } as const]
	])('is error when the %s request fails', async (_, actions, items) => {
		const { store } = setupQueue([actions], [items]);
		await store.load();
		expect(store.state).toEqual({ kind: 'error' });
	});

	it('removes an approved item and toasts that the run continues', async () => {
		const { store, approve } = setupQueue([ok([pending(1), pending(2)])], [ok([])]);
		await store.load();
		const sent = deferred<Result<PendingAction>>();
		approve.mockReturnValueOnce(sent.promise);
		const decision = store.approve(1);
		expect(store.busy.has('action:1')).toBe(true);
		expect(store.busy.has('action:2')).toBe(false);
		sent.resolve(ok(pending(1, { status: 'approved' })));
		await decision;
		expect(approve).toHaveBeenCalledWith(client, 1, undefined);
		expect(store.busy.size).toBe(0);
		expect(keys(store)).toEqual(['action:2']);
		expect(toasts()).toEqual([['success', 'Approved', 'Run #11 continues.']]);
	});

	it('words the approve toast for paused agents and passes an edited input', async () => {
		const { store, approve } = setupQueue([ok([pending(1)])], [ok([])], { paused: true });
		await store.load();
		approve.mockResolvedValueOnce(ok(pending(1)));
		await store.approve(1, { message: 'Edited' });
		expect(approve).toHaveBeenCalledWith(client, 1, { message: 'Edited' });
		expect(toasts()).toEqual([
			['success', 'Approved', 'Agents are paused, so run #11 waits until you resume them.']
		]);
	});

	it('removes a rejected action and a dismissed item with their toasts', async () => {
		const { store, reject, dismiss } = setupQueue([ok([pending(1)])], [ok([review(2), review(3)])]);
		await store.load();
		reject.mockResolvedValueOnce(ok(pending(1, { status: 'rejected' })));
		dismiss.mockResolvedValueOnce(ok(review(2, { status: 'dismissed' })));
		dismiss.mockResolvedValueOnce(ok(review(3, { status: 'dismissed' })));
		await store.reject(1);
		await store.dismiss(2, 'Not relevant');
		await store.dismiss(3);
		expect(dismiss).toHaveBeenNthCalledWith(1, client, 2, 'Not relevant');
		expect(dismiss).toHaveBeenNthCalledWith(2, client, 3, undefined);
		expect(store.items).toEqual([]);
		expect(toasts()).toEqual([
			['success', 'Rejected', 'Run #11 ended.'],
			['success', 'Dismissed', undefined],
			['success', 'Dismissed', undefined]
		]);
	});

	it.each(['conflict', 'not-found'] as const)(
		'toasts "Already decided" on %s and refreshes',
		async (kind) => {
			const { store, approve, listPending } = setupQueue(
				[ok([pending(1)]), ok([])],
				[ok([]), ok([])]
			);
			await store.load();
			approve.mockResolvedValueOnce({ kind });
			const result = await store.approve(1);
			expect(result).toEqual({ kind });
			await settle();
			expect(listPending).toHaveBeenCalledTimes(2);
			expect(store.items).toEqual([]);
			expect(toasts()).toEqual([
				['danger', 'Already decided', 'This item changed elsewhere. The queue is up to date now.']
			]);
		}
	);

	it('applies an email review item and toasts', async () => {
		const { store, accept } = setupQueue([ok([])], [ok([emailMatch(2), newApplication(3)])]);
		await store.load();
		accept.mockResolvedValueOnce(ok({ ...emailMatch(2), status: 'accepted' }));
		accept.mockResolvedValueOnce(ok({ ...newApplication(3), status: 'accepted' }));
		await store.accept(2);
		await store.accept(3);
		expect(accept).toHaveBeenNthCalledWith(1, client, 2);
		expect(accept).toHaveBeenNthCalledWith(2, client, 3);
		expect(store.items).toEqual([]);
		expect(store.busy.size).toBe(0);
		expect(toasts()).toEqual([
			['success', 'Applied to Stark Industries', undefined],
			['success', 'Added Globex', undefined]
		]);
	});

	it('keeps the item when apply conflicts', async () => {
		const { store, accept, listReview } = setupQueue(
			[ok([]), ok([])],
			[ok([emailMatch(2)]), ok([emailMatch(2)])]
		);
		await store.load();
		accept.mockResolvedValueOnce({ kind: 'conflict' });
		expect(await store.accept(2)).toEqual({ kind: 'conflict' });
		await settle();
		expect(listReview).toHaveBeenCalledTimes(2);
		expect(keys(store)).toEqual(['review:2']);
		expect(toasts()).toEqual([
			['danger', "Couldn't apply", 'The application changed or is gone since. Nothing was changed.']
		]);
	});

	it('keeps the item when the decision fails', async () => {
		const { store, dismiss } = setupQueue([ok([])], [ok([review(2)])]);
		await store.load();
		dismiss.mockResolvedValueOnce({ kind: 'failed' });
		await store.dismiss(2);
		expect(keys(store)).toEqual(['review:2']);
		expect(store.busy.size).toBe(0);
		expect(toasts()).toEqual([
			['danger', "Couldn't save your decision", 'Check that the API is running, then try again.']
		]);
	});

	it('returns 422 errors to the caller without a toast and keeps the item', async () => {
		const { store, approve } = setupQueue([ok([pending(1)])], [ok([])]);
		await store.load();
		const errors = [{ field: 'input', message: '/message: must be a string' }];
		approve.mockResolvedValueOnce({ kind: 'invalid', errors });
		expect(await store.approve(1, { message: 1 })).toEqual({ kind: 'invalid', errors });
		expect(keys(store)).toEqual(['action:1']);
		expect(toast.items).toEqual([]);
	});

	it('coalesces run.updated refreshes into at most one queued, keeping items on failure', async () => {
		const { store, listPending, listReview } = setupQueue([ok([pending(1)])], [ok([])]);
		await store.load();
		const first = deferred<Result<PendingAction[]>>();
		listPending.mockReturnValueOnce(first.promise);
		listPending.mockResolvedValueOnce({ kind: 'failed' });
		listReview.mockResolvedValue(ok([]));
		for (let i = 0; i < 4; i++) store.handle(runUpdated);
		expect(listPending).toHaveBeenCalledTimes(2);
		first.resolve(ok([pending(1), pending(2)]));
		await settle();
		expect(listPending).toHaveBeenCalledTimes(3);
		expect(keys(store)).toEqual(['action:1', 'action:2']);
		expect(store.state).toEqual({ kind: 'loaded' });
	});

	it.each(['app.connected', 'app.lagged'] as const)('refetches silently on %s', async (type) => {
		const { store } = setupQueue([ok([]), ok([pending(1)])], [ok([]), ok([])]);
		await store.load();
		store.handle({ type } as Event);
		expect(store.state).toEqual({ kind: 'loaded' });
		await settle();
		expect(keys(store)).toEqual(['action:1']);
	});

	it('empties on data.deleted, then refreshes', async () => {
		const empty = deferred<Result<PendingAction[]>>();
		const { store, listPending } = setupQueue([ok([pending(1)])], [ok([review(2)]), ok([])]);
		await store.load();
		listPending.mockReturnValueOnce(empty.promise);
		store.handle({ type: 'data.deleted' } as Event);
		expect(store.items).toEqual([]);
		empty.resolve(ok([]));
		await settle();
		expect(listPending).toHaveBeenCalledTimes(2);
		expect(store.items).toEqual([]);
	});

	it('ignores events before the first load', async () => {
		const { store, listPending } = setupQueue([ok([])], [ok([])]);
		store.handle(runUpdated);
		await settle();
		expect(listPending).not.toHaveBeenCalled();
	});
});

describe('agentsPaused', () => {
	it('is true only when the kill switch has loaded and is on', () => {
		const at = '2026-10-22T00:00:00Z';
		expect(agentsPaused({ kind: 'loaded', state: { paused: true, updated_at: at } })).toBe(true);
		expect(agentsPaused({ kind: 'loaded', state: { paused: false, updated_at: at } })).toBe(false);
		expect(agentsPaused({ kind: 'loading' })).toBe(false);
		expect(agentsPaused({ kind: 'error' })).toBe(false);
	});
});

function setupLog(...results: Result<AgentAction[]>[]) {
	const list = vi.fn<(c: unknown, options?: ListActionsOptions) => Promise<Result<AgentAction[]>>>();
	for (const result of results) list.mockResolvedValueOnce(result);
	const undo = vi.fn<(c: unknown, id: number) => Promise<Result<AgentAction>>>();
	const store = new ActionLogStore({ client, list, undo });
	return { store, list, undo };
}

const ids = (store: ActionLogStore) => store.actions.map((a) => a.id);

describe('ActionLogStore', () => {
	it('loads newest first and loads more with before', async () => {
		const { store, list } = setupLog(ok(page(120, PAGE_SIZE)), ok(page(70, 3)));
		await store.load();
		expect(list).toHaveBeenCalledWith(client, { limit: PAGE_SIZE });
		expect(store.state).toEqual({ kind: 'loaded' });
		expect(ids(store).slice(0, 2)).toEqual([120, 119]);
		expect(store.hasMore).toBe(true);
		await store.loadMore();
		expect(list).toHaveBeenLastCalledWith(client, { limit: PAGE_SIZE, before: 71 });
		expect(ids(store).slice(-3)).toEqual([70, 69, 68]);
		expect(store.hasMore).toBe(false);
	});

	it('is error when the first load fails', async () => {
		const { store } = setupLog({ kind: 'failed' });
		await store.load();
		expect(store.state).toEqual({ kind: 'error' });
	});

	it('replaces an undone entry in place and toasts', async () => {
		const { store, undo } = setupLog(ok([action(3), action(2), action(1)]));
		await store.load();
		const undone = action(2, { undone_at: '2026-10-22T01:00:00Z' });
		const sent = deferred<Result<AgentAction>>();
		undo.mockReturnValueOnce(sent.promise);
		const pendingUndo = store.undo(2);
		expect(store.busy.has(2)).toBe(true);
		sent.resolve(ok(undone));
		await pendingUndo;
		expect(store.busy.size).toBe(0);
		expect(ids(store)).toEqual([3, 2, 1]);
		expect(store.actions[1]).toEqual(undone);
		expect(toasts()).toEqual([['success', 'Undone', undefined]]);
	});

	it.each(['conflict', 'not-found'] as const)(
		'toasts "Couldn\'t undo" on %s and refreshes',
		async (kind) => {
			const refreshed = action(2, { undone_at: '2026-10-22T01:00:00Z' });
			const { store, undo, list } = setupLog(ok([action(2)]), ok([refreshed]));
			await store.load();
			undo.mockResolvedValueOnce({ kind });
			await store.undo(2);
			await settle();
			expect(list).toHaveBeenCalledTimes(2);
			expect(store.actions).toEqual([refreshed]);
			expect(toasts()).toEqual([
				[
					'danger',
					"Couldn't undo",
					'The record changed since, or this was already undone. Nothing was changed.'
				]
			]);
		}
	);

	it('leaves the entry unchanged when the undo fails', async () => {
		const { store, undo } = setupLog(ok([action(2)]));
		await store.load();
		undo.mockResolvedValueOnce({ kind: 'failed' });
		await store.undo(2);
		expect(store.actions).toEqual([action(2)]);
		expect(toasts()).toEqual([
			['danger', "Couldn't undo", 'Check that the API is running, then try again.']
		]);
	});

	it('merges a refresh by id and keeps older pages', async () => {
		const { store, list } = setupLog(
			ok(page(120, PAGE_SIZE)),
			ok(page(70, 2)),
			ok([action(121), action(120, { undone_at: '2026-10-22T01:00:00Z' })])
		);
		await store.load();
		await store.loadMore();
		store.handle(runUpdated);
		await settle();
		expect(list).toHaveBeenCalledTimes(3);
		expect(ids(store).slice(0, 2)).toEqual([121, 120]);
		expect(store.actions[1].undone_at).not.toBeNull();
		expect(ids(store).slice(-2)).toEqual([70, 69]);
	});

	it('clears on data.deleted, then refreshes', async () => {
		const { store, list } = setupLog(ok(page(120, PAGE_SIZE)), ok([]));
		await store.load();
		store.handle({ type: 'data.deleted' } as Event);
		expect(store.actions).toEqual([]);
		expect(store.hasMore).toBe(false);
		await settle();
		expect(list).toHaveBeenCalledTimes(2);
	});
});

function signal(id: number, overrides: Partial<MemorySignal> = {}): MemorySignal {
	return {
		id,
		kind: 'rejection',
		run_id: 40 + id,
		pending_action_id: id,
		review_item_id: null,
		subject: 'draft_follow_up',
		drafted: { message: 'Thanks' },
		approved: null,
		changes: null,
		reason: null,
		created_at: '2026-10-22T00:00:00Z',
		...overrides
	};
}

/** `count` signals counting down from `top`. */
function signalPage(top: number, count: number): MemorySignal[] {
	return Array.from({ length: count }, (_, i) => signal(top - i));
}

function setupSignals(...results: Result<MemorySignal[]>[]) {
	const list = vi.fn<(c: unknown, options?: ListSignalsOptions) => Promise<Result<MemorySignal[]>>>();
	for (const result of results) list.mockResolvedValueOnce(result);
	const store = new SignalLogStore({ client, list });
	return { store, list };
}

const signalIds = (store: SignalLogStore) => store.signals.map((s) => s.id);

describe('SignalLogStore', () => {
	it('loads newest first and loads more with before', async () => {
		const { store, list } = setupSignals(ok(signalPage(120, PAGE_SIZE)), ok(signalPage(70, 3)));
		expect(store.state).toEqual({ kind: 'loading' });
		await store.load();
		expect(list).toHaveBeenCalledWith(client, { limit: PAGE_SIZE });
		expect(store.state).toEqual({ kind: 'loaded' });
		expect(signalIds(store).slice(0, 2)).toEqual([120, 119]);
		expect(store.hasMore).toBe(true);
		await store.loadMore();
		expect(list).toHaveBeenLastCalledWith(client, { limit: PAGE_SIZE, before: 71 });
		expect(signalIds(store).slice(-3)).toEqual([70, 69, 68]);
		expect(store.hasMore).toBe(false);
	});

	it('is error when the first load fails', async () => {
		const { store } = setupSignals({ kind: 'failed' });
		await store.load();
		expect(store.state).toEqual({ kind: 'error' });
	});

	it('toasts and keeps the list when loading more fails', async () => {
		const { store } = setupSignals(ok(signalPage(120, PAGE_SIZE)), { kind: 'failed' });
		await store.load();
		await store.loadMore();
		expect(store.signals).toHaveLength(PAGE_SIZE);
		expect(store.loadingMore).toBe(false);
		expect(toasts()).toEqual([['danger', "Couldn't load more signals", undefined]]);
	});

	it('merges a refresh by id and keeps older pages', async () => {
		const { store, list } = setupSignals(
			ok(signalPage(120, PAGE_SIZE)),
			ok(signalPage(70, 2)),
			ok([signal(122, { kind: 'dismissal', reason: 'Wrong role' }), signal(121), signal(120)])
		);
		await store.load();
		await store.loadMore();
		store.handle(runUpdated);
		await settle();
		expect(list).toHaveBeenCalledTimes(3);
		expect(signalIds(store).slice(0, 3)).toEqual([122, 121, 120]);
		expect(store.signals[0].reason).toBe('Wrong role');
		expect(signalIds(store).slice(-2)).toEqual([70, 69]);
		expect(new Set(signalIds(store)).size).toBe(store.signals.length);
	});

	it.each(['app.connected', 'app.lagged'] as const)('refetches silently on %s', async (type) => {
		const { store } = setupSignals(ok([]), ok([signal(1)]));
		await store.load();
		store.handle({ type } as Event);
		expect(store.state).toEqual({ kind: 'loaded' });
		await settle();
		expect(signalIds(store)).toEqual([1]);
	});

	it('clears on data.deleted, then refreshes', async () => {
		const { store, list } = setupSignals(ok(signalPage(120, PAGE_SIZE)), ok([]));
		await store.load();
		store.handle({ type: 'data.deleted' } as Event);
		expect(store.signals).toEqual([]);
		expect(store.hasMore).toBe(false);
		await settle();
		expect(list).toHaveBeenCalledTimes(2);
		expect(store.signals).toEqual([]);
	});

	it('ignores refreshes before the first load', async () => {
		const { store, list } = setupSignals(ok([]));
		store.handle(runUpdated);
		await store.refresh();
		expect(list).not.toHaveBeenCalled();
	});
});
