import { SvelteSet } from 'svelte/reactivity';
import {
	createApiClient,
	type AgentAction,
	type ApiClient,
	type Event,
	type MemorySignal,
	type PendingAction,
	type ReviewItem
} from '#lib/api';
import { toast } from '#lib/components/ui';
import type { KillSwitchState } from '#lib/dashboard/dashboard.svelte.js';
import { PAGE_SIZE } from '#lib/runs/api.js';
import {
	acceptReviewItem,
	approveAction,
	dismissReviewItem,
	listActions,
	listPendingActions,
	listReviewItems,
	listSignals,
	rejectAction,
	undoAction,
	type Result
} from './api';
import { describeEmailReview } from './email';
import { describeFitReview } from './fit';
import { describePacketReview } from './packet';

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
const APPLY_CONFLICT = {
	title: "Couldn't apply",
	description: 'The application changed or is gone since. Nothing was changed.'
};

const FIT_APPLY_CONFLICT = {
	title: "Couldn't apply",
	description: 'This score is no longer waiting for review. Nothing was changed.'
};

const PACKET_APPLY_CONFLICT = {
	title: "Couldn't apply",
	description: 'This packet is no longer waiting for review. Nothing was changed.'
};

type Notice = { title: string; description: string };

export interface QueueDeps {
	client: ApiClient;
	listPending: typeof listPendingActions;
	listReview: typeof listReviewItems;
	approve: typeof approveAction;
	reject: typeof rejectAction;
	dismiss: typeof dismissReviewItem;
	accept: typeof acceptReviewItem;
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
			accept: deps.accept ?? acceptReviewItem,
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
		onOk: () => void,
		conflict: Notice = ALREADY_DECIDED
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
				toast.error(conflict.title, { description: conflict.description });
				void this.refresh();
				break;
			case 'not-found':
				toast.error(ALREADY_DECIDED.title, { description: ALREADY_DECIDED.description });
				void this.refresh();
				break;
			case 'failed':
				toast.error(SAVE_FAILED.title, { description: SAVE_FAILED.description });
				break;
			case 'invalid':
				// The Edit and Dismiss dialogs show these under their field.
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

	/** Dismisses a review item, with the user's reason when given; nothing is applied. */
	dismiss(id: number, reason?: string): Promise<Result<ReviewItem>> {
		const { client, dismiss } = this.#deps;
		return this.#decide(
			`review:${id}`,
			() => dismiss(client, id, reason),
			() => toast.success('Dismissed')
		);
	}

	/**
	 * Applies an email review item (`email.match` or `email.new_application`) as the user's own
	 * change, makes a `score_fit.score` item's score its posting's current one, or makes a
	 * `prep_packet.packet` item's packet ready as it is. A conflict (the application changed or
	 * is gone, or the score or packet is no longer in review) keeps the item and says so.
	 */
	accept(id: number): Promise<Result<ReviewItem>> {
		const key = `review:${id}`;
		const entry = this.items.find((held) => queueKey(held) === key);
		const item = entry?.kind === 'review' ? entry.item : null;
		const summary = item ? describeEmailReview(item) : null;
		const fit = item ? describeFitReview(item) : null;
		const packet = item ? describePacketReview(item) : null;
		const { client, accept } = this.#deps;
		return this.#decide(
			key,
			() => accept(client, id),
			() => {
				if (fit) toast.success(`Applied fit score for ${fit.company}`);
				else if (packet) toast.success(`Packet ready for ${packet.company}`);
				else if (!summary) toast.success('Applied');
				else if (summary.kind === 'new_application') toast.success(`Added ${summary.company}`);
				else toast.success(`Applied to ${summary.company}`);
			},
			fit ? FIT_APPLY_CONFLICT : packet ? PACKET_APPLY_CONFLICT : APPLY_CONFLICT
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

/** Fetches one page of a log, newest first. */
export type ListPage<T> = (
	client: ApiClient,
	options: { limit: number; before?: number }
) => Promise<Result<T[]>>;

/**
 * A read-mostly log loaded newest first a page at a time: the first page with a skeleton,
 * more pages on demand, and silent refreshes of the first page merged by id on `run.updated`
 * and after a reconnect or lag. `data.deleted` clears it, then refreshes.
 */
class PagedLog<T extends { id: number }> {
	state = $state<ListState>({ kind: 'loading' });
	/** Entries held, `id` descending. */
	items = $state.raw<T[]>([]);
	/** Whether the last page was full, so another may follow. */
	hasMore = $state(false);
	loadingMore = $state(false);

	#client: ApiClient;
	#list: ListPage<T>;
	#loadMoreError: string;
	#refresher = new Coalescer(() => this.#fetchFirstPage());

	constructor(client: ApiClient, list: ListPage<T>, loadMoreError: string) {
		this.#client = client;
		this.#list = list;
		this.#loadMoreError = loadMoreError;
	}

	/** Loads the first page, showing the skeleton meanwhile. */
	async load(): Promise<void> {
		this.state = { kind: 'loading' };
		const result = await this.#list(this.#client, { limit: PAGE_SIZE });
		if (result.kind === 'ok') {
			this.items = result.data;
			this.hasMore = result.data.length === PAGE_SIZE;
			this.state = { kind: 'loaded' };
		} else {
			this.state = { kind: 'error' };
		}
	}

	/** Appends the next page: the entries below the smallest id held. */
	async loadMore(): Promise<void> {
		if (this.state.kind !== 'loaded' || this.loadingMore || !this.hasMore) return;
		const oldest = this.items.at(-1);
		if (!oldest) return;
		this.loadingMore = true;
		try {
			const result = await this.#list(this.#client, { limit: PAGE_SIZE, before: oldest.id });
			if (result.kind !== 'ok') {
				toast.error(this.#loadMoreError);
				return;
			}
			const held = new Set(this.items.map((entry) => entry.id));
			this.items = [...this.items, ...result.data.filter((entry) => !held.has(entry.id))];
			this.hasMore = result.data.length === PAGE_SIZE;
		} finally {
			this.loadingMore = false;
		}
	}

	async #fetchFirstPage(): Promise<void> {
		const result = await this.#list(this.#client, { limit: PAGE_SIZE });
		if (result.kind !== 'ok') return;
		// A local lookup, not reactive state.
		// eslint-disable-next-line svelte/prefer-svelte-reactivity
		const merged = new Map(this.items.map((entry) => [entry.id, entry]));
		for (const entry of result.data) merged.set(entry.id, entry);
		this.items = [...merged.values()].toSorted((a, b) => b.id - a.id);
	}

	/**
	 * Refetches the first page without the skeleton and merges it by id, keeping older pages.
	 * Keeps the current data when it fails. Coalesced, and does nothing before the first load.
	 */
	refresh(): Promise<void> {
		if (this.state.kind !== 'loaded') return Promise.resolve();
		return this.#refresher.run();
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'data.deleted':
				// Every entry is gone, so the older pages a refresh keeps must go too.
				if (this.state.kind === 'loaded') {
					this.items = [];
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

export interface ActionLogDeps {
	client: ApiClient;
	list: typeof listActions;
	undo: typeof undoAction;
}

/** The action log: `GET /actions` as a {@link PagedLog}, with undo. */
export class ActionLogStore extends PagedLog<AgentAction> {
	/** Ids of the actions with an undo in flight. */
	busy = new SvelteSet<number>();

	#client: ApiClient;
	#undo: typeof undoAction;

	constructor(deps: Partial<ActionLogDeps> = {}) {
		const client = deps.client ?? createApiClient();
		super(client, deps.list ?? listActions, "Couldn't load more actions");
		this.#client = client;
		this.#undo = deps.undo ?? undoAction;
	}

	/** Actions held, `id` descending. */
	get actions(): AgentAction[] {
		return this.items;
	}

	/** Undoes one action; on success its entry is replaced in place by the undone one. */
	async undo(id: number): Promise<Result<AgentAction>> {
		if (this.busy.has(id)) return { kind: 'failed' };
		this.busy.add(id);
		let result: Result<AgentAction>;
		try {
			result = await this.#undo(this.#client, id);
		} finally {
			this.busy.delete(id);
		}
		switch (result.kind) {
			case 'ok': {
				const undone = result.data;
				this.items = this.items.map((action) => (action.id === id ? undone : action));
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
}

export interface SignalLogDeps {
	client: ApiClient;
	list: typeof listSignals;
}

/** The learning signals: `GET /memory-signals` as a read-only {@link PagedLog}. */
export class SignalLogStore extends PagedLog<MemorySignal> {
	constructor(deps: Partial<SignalLogDeps> = {}) {
		super(deps.client ?? createApiClient(), deps.list ?? listSignals, "Couldn't load more signals");
	}

	/** Signals held, `id` descending. */
	get signals(): MemorySignal[] {
		return this.items;
	}
}
