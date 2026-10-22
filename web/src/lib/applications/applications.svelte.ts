import { SvelteSet } from 'svelte/reactivity';
import {
	createApiClient,
	type ApiClient,
	type Application,
	type ApplicationStatus,
	type Event
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { listApplications, setApplicationStatus } from './api';
import { compareApplications } from './board';
import { STATUSES, statusLabel } from './statuses';

/** Whether the list has loaded. `loading` shows the skeleton; a silent refresh never does. */
export type ApplicationsState = { kind: 'loading' } | { kind: 'error' } | { kind: 'loaded' };

/** One board column: a status and its applications in list order. */
export interface BoardColumnData {
	status: ApplicationStatus;
	applications: Application[];
}

export interface ApplicationsDeps {
	client: ApiClient;
	list: typeof listApplications;
	setStatus: typeof setApplicationStatus;
}

/** Of a held row and an incoming one, whether the incoming one wins: later or equal `updated_at`. */
function newer(incoming: Application, held: Application | undefined): boolean {
	return !held || Date.parse(incoming.updated_at) >= Date.parse(held.updated_at);
}

/**
 * The Applications page's live state: the list from `GET /applications`, kept current by
 * `application.*` events and refetched silently after a reconnect or lag, plus optimistic
 * status moves for the board.
 */
export class ApplicationsStore {
	state = $state<ApplicationsState>({ kind: 'loading' });

	// Raw and replaced on every change, so a held row keeps its identity: a failed move
	// reverts only its own optimistic row.
	#rows = $state.raw<Application[]>([]);
	#pending = new SvelteSet<number>();
	#deps: ApplicationsDeps;
	#refreshing = false;

	/** Every application, newest `updated_at` first, ties by `id` descending. */
	applications = $derived(this.#rows.toSorted(compareApplications));
	/** The five board columns, in board order. */
	columns: BoardColumnData[] = $derived(
		STATUSES.map((status) => ({
			status,
			applications: this.applications.filter((application) => application.status === status)
		}))
	);

	constructor(deps: Partial<ApplicationsDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			list: deps.list ?? listApplications,
			setStatus: deps.setStatus ?? setApplicationStatus
		};
	}

	/** Ids whose move request is in flight. */
	get pending(): ReadonlySet<number> {
		return this.#pending;
	}

	#get(id: number): Application | undefined {
		return this.#rows.find((application) => application.id === id);
	}

	#set(application: Application): void {
		this.#rows = [...this.#rows.filter((held) => held.id !== application.id), application];
	}

	/** Loads the list, showing the skeleton meanwhile. */
	async load(): Promise<void> {
		this.state = { kind: 'loading' };
		const result = await this.#deps.list(this.#deps.client);
		if (result.kind === 'ok') {
			this.#rows = result.data;
			this.state = { kind: 'loaded' };
		} else {
			this.state = { kind: 'error' };
		}
	}

	/**
	 * Refetches the list without the skeleton. Keeps the current data when it fails, and
	 * does nothing before the first load or while a refresh is running.
	 */
	async refresh(): Promise<void> {
		if (this.state.kind !== 'loaded' || this.#refreshing) return;
		this.#refreshing = true;
		try {
			const result = await this.#deps.list(this.#deps.client);
			if (result.kind !== 'ok') return;
			// A row that's moving keeps its optimistic copy until its request settles.
			this.#rows = result.data.map((application) =>
				this.pending.has(application.id) ? (this.#get(application.id) ?? application) : application
			);
		} finally {
			this.#refreshing = false;
		}
	}

	/** Inserts `application`, or replaces the held copy unless that one is newer. */
	upsert(application: Application): void {
		if (this.state.kind !== 'loaded') return;
		if (newer(application, this.#get(application.id))) this.#set(application);
	}

	remove(id: number): void {
		if (this.state.kind !== 'loaded' || !this.#get(id)) return;
		this.#rows = this.#rows.filter((application) => application.id !== id);
	}

	/** Applies one event from `/events`. Events before the first load finishes are dropped. */
	apply(event: Event): void {
		switch (event.type) {
			case 'app.connected':
			case 'app.lagged':
				void this.refresh();
				return;
			case 'application.created':
			case 'application.updated':
				this.upsert(event.application);
				return;
			case 'application.deleted':
				this.remove(event.id);
				return;
		}
	}

	/**
	 * Moves an application to `status`: at once on screen, then through the API. Resolves
	 * `true` once the API stored it. A move to the current status, or of an application
	 * that's already moving, does nothing and resolves `false`.
	 */
	async move(id: number, status: ApplicationStatus): Promise<boolean> {
		const before = this.#get(id);
		if (!before || before.status === status || this.pending.has(id)) return false;
		// A plain timestamp, not reactive state.
		// eslint-disable-next-line svelte/prefer-svelte-reactivity
		const now = new Date().toISOString();
		const optimistic: Application = {
			...before,
			status,
			status_changed_at: now,
			updated_at: now
		};
		this.#set(optimistic);
		this.#pending.add(id);

		const result = await this.#deps.setStatus(this.#deps.client, id, status);
		this.#pending.delete(id);
		if (result.kind === 'ok') {
			// Deleted meanwhile: don't bring it back.
			if (this.#get(id)) this.#set(result.data);
			toast.success(`Moved to ${statusLabel[result.data.status]}`);
			return true;
		}
		// A newer row from an event replaced the optimistic one: keep it.
		if (this.#get(id) === optimistic) this.#set(before);
		toast.error(`Couldn't move ${before.company} · ${before.title}`);
		return false;
	}
}
