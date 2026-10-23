import { createApiClient, type ApiClient, type Event, type Run, type RunDetail } from '#lib/api';
import { toast } from '#lib/components/ui';
import { getRun, listRuns, PAGE_SIZE } from './api';

/** Whether the list has loaded. `loading` shows the skeleton; a silent refresh never does. */
export type RunsState = { kind: 'loading' } | { kind: 'error' } | { kind: 'loaded' };

export interface RunsDeps {
	client: ApiClient;
	list: typeof listRuns;
}

/** Of a held run and an incoming one, whether the incoming one wins: later or equal `updated_at`. */
function newer(incoming: Run, held: Run): boolean {
	return Date.parse(incoming.updated_at) >= Date.parse(held.updated_at);
}

const byIdDescending = (a: Run, b: Run) => b.id - a.id;

/**
 * The Runs page's live state: `GET /runs` a page at a time, kept current by `run.updated`
 * events and refetched silently after a reconnect or lag.
 */
export class RunsStore {
	state = $state<RunsState>({ kind: 'loading' });
	/** Runs held, `id` descending. */
	runs = $state.raw<Run[]>([]);
	/** Whether the last page was full, so another may follow. */
	hasMore = $state(false);
	loadingMore = $state(false);

	#deps: RunsDeps;
	#refreshing = false;

	constructor(deps: Partial<RunsDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			list: deps.list ?? listRuns
		};
	}

	/** Loads the first page, showing the skeleton meanwhile. */
	async load(): Promise<void> {
		this.state = { kind: 'loading' };
		const result = await this.#deps.list(this.#deps.client, {
			limit: PAGE_SIZE
		});
		if (result.kind === 'ok') {
			this.runs = result.data;
			this.hasMore = result.data.length === PAGE_SIZE;
			this.state = { kind: 'loaded' };
		} else {
			this.state = { kind: 'error' };
		}
	}

	/** Appends the next page: the runs below the smallest id held. */
	async loadMore(): Promise<void> {
		if (this.state.kind !== 'loaded' || this.loadingMore || !this.hasMore) return;
		const oldest = this.runs.at(-1);
		if (!oldest) return;
		this.loadingMore = true;
		try {
			const result = await this.#deps.list(this.#deps.client, {
				limit: PAGE_SIZE,
				before: oldest.id
			});
			if (result.kind !== 'ok') {
				toast.error("Couldn't load more runs");
				return;
			}
			const held = this.runs.map((run) => run.id);
			this.runs = [...this.runs, ...result.data.filter((run) => !held.includes(run.id))];
			this.hasMore = result.data.length === PAGE_SIZE;
		} finally {
			this.loadingMore = false;
		}
	}

	/**
	 * Refetches the first page without the skeleton and merges it, keeping older pages. Keeps
	 * the current data when it fails, and does nothing before the first load or while running.
	 */
	async refresh(): Promise<void> {
		if (this.state.kind !== 'loaded' || this.#refreshing) return;
		this.#refreshing = true;
		try {
			const result = await this.#deps.list(this.#deps.client, { limit: PAGE_SIZE });
			if (result.kind !== 'ok') return;
			// A local lookup, not reactive state.
			// eslint-disable-next-line svelte/prefer-svelte-reactivity
			const merged = new Map(this.runs.map((run) => [run.id, run]));
			for (const run of result.data) {
				const held = merged.get(run.id);
				if (!held || newer(run, held)) merged.set(run.id, run);
			}
			this.runs = [...merged.values()].toSorted(byIdDescending);
		} finally {
			this.#refreshing = false;
		}
	}

	/**
	 * Applies a run from an event: replaces the held copy unless that one is newer, puts a run
	 * newer than every held one on top, and ignores a run older than the oldest held.
	 */
	upsert(run: Run): void {
		if (this.state.kind !== 'loaded') return;
		const index = this.runs.findIndex((held) => held.id === run.id);
		if (index >= 0) {
			if (newer(run, this.runs[index])) this.runs = this.runs.with(index, run);
			return;
		}
		const oldest = this.runs.at(-1);
		if (oldest && run.id < oldest.id) return;
		this.runs = [run, ...this.runs].toSorted(byIdDescending);
	}

	/** Applies one event from `/events`. Events before the first load finishes are dropped. */
	handle(event: Event): void {
		switch (event.type) {
			case 'data.deleted':
				// Every run is gone, so the older pages a refresh keeps must go too.
				if (this.state.kind === 'loaded') {
					this.runs = [];
					this.hasMore = false;
				}
				void this.refresh();
				return;
			case 'app.connected':
			case 'app.lagged':
				void this.refresh();
				return;
			case 'run.updated':
				this.upsert(event.run);
				return;
		}
	}
}

export type RunDetailState =
	| { kind: 'loading' }
	| { kind: 'not-found' }
	| { kind: 'error' }
	| { kind: 'loaded'; run: RunDetail };

export interface RunDetailDeps {
	client: ApiClient;
	get: typeof getRun;
}

/**
 * One run's page: `GET /runs/{id}`, refetched whenever `run.updated` names this run and after
 * a reconnect or lag. Refetches are coalesced: one request in flight and at most one queued.
 */
export class RunDetailStore {
	state = $state<RunDetailState>({ kind: 'loading' });

	readonly id: number;
	#deps: RunDetailDeps;
	#inFlight: Promise<void> | null = null;
	#queued = false;

	constructor(id: number, deps: Partial<RunDetailDeps> = {}) {
		this.id = id;
		this.#deps = {
			client: deps.client ?? createApiClient(),
			get: deps.get ?? getRun
		};
	}

	/** The run, once loaded. */
	get run(): RunDetail | null {
		return this.state.kind === 'loaded' ? this.state.run : null;
	}

	/** Loads the run, showing the skeleton meanwhile. */
	async load(): Promise<void> {
		this.state = { kind: 'loading' };
		const result = await this.#deps.get(this.#deps.client, this.id);
		if (result.kind === 'ok') this.state = { kind: 'loaded', run: result.data };
		else
			this.state = {
				kind: result.kind === 'not-found' ? 'not-found' : 'error'
			};
	}

	/**
	 * Refetches without the skeleton, keeping the run shown when it fails. A call while one is
	 * in flight queues one more; further calls join that queued one.
	 */
	refresh(): Promise<void> {
		if (this.state.kind !== 'loaded') return Promise.resolve();
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
			const result = await this.#deps.get(this.#deps.client, this.id);
			if (result.kind === 'ok') this.state = { kind: 'loaded', run: result.data };
		} while (this.#queued);
	}

	/** Applies one event from `/events`: only this run's updates and reconnects refetch. */
	handle(event: Event): void {
		switch (event.type) {
			case 'data.deleted':
				// A refresh keeps the shown run when it's gone; a load shows "not found".
				if (this.state.kind === 'loaded') void this.load();
				return;
			case 'app.connected':
			case 'app.lagged':
				void this.refresh();
				return;
			case 'run.updated':
				if (event.run.id === this.id) void this.refresh();
				return;
		}
	}
}
