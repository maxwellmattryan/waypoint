import { SvelteSet } from 'svelte/reactivity';
import {
	createApiClient,
	type AgentAction,
	type ApiClient,
	type Event,
	type PendingAction,
	type ReviewItem
} from '#lib/api';
import { toast } from '#lib/components/ui';
import type { KillSwitchState } from '#lib/dashboard/dashboard.svelte.js';
import { PAGE_SIZE } from '#lib/runs/api.js';
import {
	approveAction,
	dismissReviewItem,
	listActions,
	listPendingActions,
	listReviewItems,
	rejectAction,
	undoAction,
	type Result
} from './api';

/** Whether the list has loaded. `loading` shows the skeleton; a silent refresh never does. */
export type ListState = { kind: 'loading' } | { kind: 'error' } | { kind: 'loaded' };

/** One entry in the Review queue: an action at the approval gate, or a low-confidence result. */
export type QueueItem =
	| { kind: 'action'; action: PendingAction }
	| { kind: 'review'; item: ReviewItem };

/** Whether agents are paused: the kill switch has loaded and is on. */
export function agentsPaused(killSwitch: KillSwitchState): boolean {
	return killSwitch.kind === 'loaded' && killSwitch.state.paused;
}

/** A queue item's stable key: `action:<id>` or `review:<id>`. */
export function queueKey(entry: QueueItem): string {
	return entry.kind === 'action' ? `action:${entry.action.id}` : `review:${entry.item.id}`;
}

const record = (entry: QueueItem) => (entry.kind === 'action' ? entry.action : entry.item);

/** Oldest first; on equal timestamps pending actions come first, then lower ids. */
function byQueueOrder(a: QueueItem, b: QueueItem): number {
	const time = Date.parse(record(a).created_at) - Date.parse(record(b).created_at);
	if (time !== 0) return time;
	if (a.kind !== b.kind) return a.kind === 'action' ? -1 : 1;
	return record(a).id - record(b).id;
}

/** Pending actions and review items as one queue, oldest first. */
export function mergeQueue(actions: PendingAction[], items: ReviewItem[]): QueueItem[] {
	return [
		...actions.map((action): QueueItem => ({ kind: 'action', action })),
		...items.map((item): QueueItem => ({ kind: 'review', item }))
	].toSorted(byQueueOrder);
}

/**
 * Runs `fetch` coalesced: one call in flight and at most one queued; further calls join the
 * queued one.
 */
class Coalescer {
	#fetch: () => Promise<void>;
	#inFlight: Promise<void> | null = null;
	#queued = false;

	constructor(fetch: () => Promise<void>) {
		this.#fetch = fetch;
	}

	run(): Promise<void> {
		if (this.#inFlight) {
			this.#queued = true;
			return this.#inFlight;
		}
		this.#inFlight = this.#drain().finally(() => (this.#inFlight = null));
		return this.#inFlight;
	}

	async #drain(): Promise<void> {
		do {
			this.#queued = false;
			await this.#fetch();
		} while (this.#queued);
	}
}

const ALREADY_DECIDED = {
	title: 'Already decided',
	description: 'This item changed elsewhere. The queue is up to date now.'
};
const SAVE_FAILED = {
	title: "Couldn't save your decision",
	description: 'Check that the API is running, then try again.'
};

export interface QueueDeps {
	client: ApiClient;
	listPending: typeof listPendingActions;
	listReview: typeof listReviewItems;
	approve: typeof approveAction;
	reject: typeof rejectAction;
	dismiss: typeof dismissReviewItem;
	/** Whether agents are paused, for the approve toast's wording. */
	paused: () => boolean;
}

/**
 * The Review queue: every `pending` action and review item, refetched silently on
 * `run.updated` and after a reconnect or lag. The store owns the decisions' toasts.
 */
export class QueueStore {
	state = $state<ListState>({ kind: 'loading' });
	/** The queue, oldest first. */
	items = $state.raw<QueueItem[]>([]);
	/** Keys of the items with a decision in flight. */
	busy = new SvelteSet<string>();

	#deps: QueueDeps;
	#refresher = new Coalescer(() => this.#fetchSilently());

	constructor(deps: Partial<QueueDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			listPending: deps.listPending ?? listPendingActions,
			listReview: deps.listReview ?? listReviewItems,
			approve: deps.approve ?? approveAction,
			reject: deps.reject ?? rejectAction,
			dismiss: deps.dismiss ?? dismissReviewItem,
			paused: deps.paused ?? (() => false)
		};
	}

	async #fetch(): Promise<QueueItem[] | null> {
		const { client, listPending, listReview } = this.#deps;
		const [actions, items] = await Promise.all([listPending(client), listReview(client)]);
		if (actions.kind !== 'ok' || items.kind !== 'ok') return null;
		return mergeQueue(actions.data, items.data);
	}

	async #fetchSilently(): Promise<void> {
		const items = await this.#fetch();
		if (items) this.items = items;
	}

	/** Loads both lists, showing the skeleton meanwhile; `error` when either fails. */
	async load(): Promise<void> {
		this.state = { kind: 'loading' };
		const items = await this.#fetch();
		if (items) {
			this.items = items;
			this.state = { kind: 'loaded' };
		} else {
			this.state = { kind: 'error' };
		}
	}

	/**
	 * Refetches both lists without the skeleton, keeping the queue when it fails. Coalesced,
	 * and does nothing before the first load.
	 */
	refresh(): Promise<void> {
		if (this.state.kind !== 'loaded') return Promise.resolve();
		return this.#refresher.run();
	}

	async #decide<T>(
		key: string,
		send: () => Promise<Result<T>>,
		onOk: () => void
	): Promise<Result<T>> {
		if (this.busy.has(key)) return { kind: 'failed' };
		this.busy.add(key);
		let result: Result<T>;
		try {
			result = await send();
		} finally {
			this.busy.delete(key);
		}
		switch (result.kind) {
			case 'ok':
				this.items = this.items.filter((entry) => queueKey(entry) !== key);
				onOk();
				break;
			case 'conflict':
			case 'not-found':
				toast.error(ALREADY_DECIDED.title, { description: ALREADY_DECIDED.description });
				void this.refresh();
				break;
			case 'failed':
				toast.error(SAVE_FAILED.title, { description: SAVE_FAILED.description });
				break;
			case 'invalid':
				// The Edit dialog shows these under its field.
				break;
		}
		return result;
	}

	#runOf(key: string): number | undefined {
		const entry = this.items.find((held) => queueKey(held) === key);
		return entry && record(entry).run_id;
	}

	/** Approves a pending action, with `input` replacing the requested one when given. */
	approve(id: number, input?: Record<string, unknown>): Promise<Result<PendingAction>> {
		const key = `action:${id}`;
		const run = this.#runOf(key);
		const { client, approve, paused } = this.#deps;
		return this.#decide(
			key,
			() => approve(client, id, input),
			() =>
				toast.success('Approved', {
					description: paused()
						? `Agents are paused, so run #${run} waits until you resume them.`
						: `Run #${run} continues.`
				})
		);
	}

	/** Rejects a pending action, ending its run. */
	reject(id: number): Promise<Result<PendingAction>> {
		const key = `action:${id}`;
		const run = this.#runOf(key);
		const { client, reject } = this.#deps;
		return this.#decide(
			key,
			() => reject(client, id),
			() => toast.success('Rejected', { description: `Run #${run} ended.` })
		);
	}

	/** Dismisses a review item; nothing is applied. */
	dismiss(id: number): Promise<Result<ReviewItem>> {
		const { client, dismiss } = this.#deps;
		return this.#decide(
			`review:${id}`,
			() => dismiss(client, id),
			() => toast.success('Dismissed')
		);
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'data.deleted':
				if (this.state.kind === 'loaded') this.items = [];
				void this.refresh();
				return;
			case 'run.updated':
			case 'app.connected':
			case 'app.lagged':
				void this.refresh();
				return;
		}
	}
}

export interface ActionLogDeps {
	client: ApiClient;
	list: typeof listActions;
	undo: typeof undoAction;
}

const byIdDescending = (a: AgentAction, b: AgentAction) => b.id - a.id;

/**
 * The action log: `GET /actions` newest first a page at a time, with undo, refetched
 * silently on `run.updated` and after a reconnect or lag.
 */
export class ActionLogStore {
	state = $state<ListState>({ kind: 'loading' });
	/** Actions held, `id` descending. */
	actions = $state.raw<AgentAction[]>([]);
	/** Whether the last page was full, so another may follow. */
	hasMore = $state(false);
	loadingMore = $state(false);
	/** Ids of the actions with an undo in flight. */
	busy = new SvelteSet<number>();

	#deps: ActionLogDeps;
	#refresher = new Coalescer(() => this.#fetchFirstPage());

	constructor(deps: Partial<ActionLogDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			list: deps.list ?? listActions,
			undo: deps.undo ?? undoAction
		};
	}

	/** Loads the first page, showing the skeleton meanwhile. */
	async load(): Promise<void> {
		this.state = { kind: 'loading' };
		const result = await this.#deps.list(this.#deps.client, { limit: PAGE_SIZE });
		if (result.kind === 'ok') {
			this.actions = result.data;
			this.hasMore = result.data.length === PAGE_SIZE;
			this.state = { kind: 'loaded' };
		} else {
			this.state = { kind: 'error' };
		}
	}

	/** Appends the next page: the actions below the smallest id held. */
	async loadMore(): Promise<void> {
		if (this.state.kind !== 'loaded' || this.loadingMore || !this.hasMore) return;
		const oldest = this.actions.at(-1);
		if (!oldest) return;
		this.loadingMore = true;
		try {
			const result = await this.#deps.list(this.#deps.client, {
				limit: PAGE_SIZE,
				before: oldest.id
			});
			if (result.kind !== 'ok') {
				toast.error("Couldn't load more actions");
				return;
			}
			// A local lookup, not reactive state.
			// eslint-disable-next-line svelte/prefer-svelte-reactivity
			const held = new Set(this.actions.map((action) => action.id));
			this.actions = [...this.actions, ...result.data.filter((action) => !held.has(action.id))];
			this.hasMore = result.data.length === PAGE_SIZE;
		} finally {
			this.loadingMore = false;
		}
	}

	async #fetchFirstPage(): Promise<void> {
		const result = await this.#deps.list(this.#deps.client, { limit: PAGE_SIZE });
		if (result.kind !== 'ok') return;
		// A local lookup, not reactive state.
		// eslint-disable-next-line svelte/prefer-svelte-reactivity
		const merged = new Map(this.actions.map((action) => [action.id, action]));
		for (const action of result.data) merged.set(action.id, action);
		this.actions = [...merged.values()].toSorted(byIdDescending);
	}

	/**
	 * Refetches the first page without the skeleton and merges it by id, keeping older pages.
	 * Keeps the current data when it fails. Coalesced, and does nothing before the first load.
	 */
	refresh(): Promise<void> {
		if (this.state.kind !== 'loaded') return Promise.resolve();
		return this.#refresher.run();
	}

	/** Undoes one action; on success its entry is replaced in place by the undone one. */
	async undo(id: number): Promise<Result<AgentAction>> {
		if (this.busy.has(id)) return { kind: 'failed' };
		this.busy.add(id);
		let result: Result<AgentAction>;
		try {
			result = await this.#deps.undo(this.#deps.client, id);
		} finally {
			this.busy.delete(id);
		}
		switch (result.kind) {
			case 'ok': {
				const undone = result.data;
				this.actions = this.actions.map((action) => (action.id === id ? undone : action));
				toast.success('Undone');
				break;
			}
			case 'conflict':
			case 'not-found':
				toast.error("Couldn't undo", {
					description: 'The record changed since, or this was already undone. Nothing was changed.'
				});
				void this.refresh();
				break;
			default:
				toast.error("Couldn't undo", {
					description: 'Check that the API is running, then try again.'
				});
		}
		return result;
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'data.deleted':
				// Every action is gone, so the older pages a refresh keeps must go too.
				if (this.state.kind === 'loaded') {
					this.actions = [];
					this.hasMore = false;
				}
				void this.refresh();
				return;
			case 'run.updated':
			case 'app.connected':
			case 'app.lagged':
				void this.refresh();
				return;
		}
	}
}
