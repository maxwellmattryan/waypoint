import {
	createApiClient,
	type ApiClient,
	type Event,
	type JobPosting,
	type JobPostingDetail,
	type PostingFit,
	type Source
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { fitView, getPostingFit, SCORE_FIT_WORKFLOW, scorePosting, type FitView } from './fit';
import {
	EMPTY_FILTERS,
	getPosting,
	hasFilters,
	listPostings,
	PAGE_SIZE,
	type PostingFilters
} from './api';

/** How long the text filters wait after the last keystroke before they reload. */
export const FILTER_DEBOUNCE_MS = 300;

/** The `source_board` job kind, whose success means the postings changed. */
const SOURCE_BOARD = 'source_board';

/** Whether the list has loaded. `loading` shows the skeleton; later reloads keep the list. */
export type PostingsView = { kind: 'loading' } | { kind: 'error' } | { kind: 'loaded' };

export interface PostingsDeps {
	client: ApiClient;
	list: typeof listPostings;
}

/** One count from a `source_board` job's result; a missing or odd one reads 0. */
function count(result: unknown, key: string): number {
	if (typeof result !== 'object' || result === null) return 0;
	const value = (result as Record<string, unknown>)[key];
	return typeof value === 'number' ? value : 0;
}

/**
 * The Postings page's list: `GET /postings` with the current filters a page at a time. A filter
 * change reloads from the first page (the text filters after a 300 ms pause), a response for an
 * older filter set is dropped, and `/events` keeps it current. Toasts the outcome of each board
 * fetch the page started.
 */
export class PostingsStore {
	filters = $state<PostingFilters>({ ...EMPTY_FILTERS });
	view = $state<PostingsView>({ kind: 'loading' });
	/** The postings held, `id` descending. */
	postings = $state.raw<JobPosting[]>([]);
	/** Whether the last page was full, so another may follow. */
	hasMore = $state(false);
	loadingMore = $state(false);

	#deps: PostingsDeps;
	/** Bumped per first-page request; a response for an older one is dropped. */
	#request = 0;
	/** Whether a first-page load has succeeded; silent refreshes wait for it. */
	#loadedOnce = false;
	#debounce: ReturnType<typeof setTimeout> | undefined;
	/** The board slug of each `source_board` job this page queued, until it finishes. */
	// A plain lookup, not reactive state.
	// eslint-disable-next-line svelte/prefer-svelte-reactivity
	#tracked = new Map<number, string>();

	constructor(deps: Partial<PostingsDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			list: deps.list ?? listPostings
		};
	}

	/** Whether any filter is set. */
	get filtered(): boolean {
		return hasFilters(this.filters);
	}

	/**
	 * Loads the first page with the current filters. The skeleton shows only until the first load
	 * succeeds; later loads keep the list until the new one arrives.
	 */
	async load(): Promise<void> {
		this.#cancelDebounce();
		const request = ++this.#request;
		if (this.view.kind !== 'loaded') this.view = { kind: 'loading' };
		const result = await this.#firstPage();
		if (request !== this.#request) return;
		if (result.kind === 'ok') {
			this.#replace(result.data);
			this.#loadedOnce = true;
		} else {
			this.view = { kind: 'error' };
		}
	}

	/** Appends the next page: the postings below the smallest id held. */
	async loadMore(): Promise<void> {
		if (this.view.kind !== 'loaded' || this.loadingMore || !this.hasMore) return;
		const oldest = this.postings.at(-1);
		if (!oldest) return;
		const request = this.#request;
		this.loadingMore = true;
		try {
			const result = await this.#deps.list(
				this.#deps.client,
				{ ...this.filters },
				{ limit: PAGE_SIZE, before: oldest.id }
			);
			// The filters changed or the list reloaded meanwhile: this page belongs to the old list.
			if (request !== this.#request) return;
			if (result.kind !== 'ok') {
				toast.error("Couldn't load more postings");
				return;
			}
			const held = this.postings.map((posting) => posting.id);
			this.postings = [
				...this.postings,
				...result.data.filter((posting) => !held.includes(posting.id))
			];
			this.hasMore = result.data.length === PAGE_SIZE;
		} finally {
			this.loadingMore = false;
		}
	}

	/**
	 * Replaces the list with the first page for the current filters, without the skeleton. Drops
	 * pages added with "Load more". Keeps what's shown when it fails, and does nothing until a
	 * first load has succeeded. A success after a failed reload shows the list again.
	 */
	async refresh(): Promise<void> {
		if (!this.#loadedOnce) return;
		const request = ++this.#request;
		const result = await this.#firstPage();
		if (request !== this.#request || result.kind !== 'ok') return;
		this.#replace(result.data);
	}

	/** Sets the company filter; reloads 300 ms after the last call. */
	setCompany(company: string): void {
		this.filters.company = company;
		this.#debounced();
	}

	/** Sets the location filter; reloads 300 ms after the last call. */
	setLocation(location: string): void {
		this.filters.location = location;
		this.#debounced();
	}

	setSource(source: Source | ''): Promise<void> {
		this.filters.source = source;
		return this.load();
	}

	setRemote(remote: PostingFilters['remote']): Promise<void> {
		this.filters.remote = remote;
		return this.load();
	}

	setIncludeRemoved(includeRemoved: boolean): Promise<void> {
		this.filters.includeRemoved = includeRemoved;
		return this.load();
	}

	clearFilters(): Promise<void> {
		this.filters = { ...EMPTY_FILTERS };
		return this.load();
	}

	/** Remembers a `source_board` job this page queued, so its outcome toasts once. */
	track(jobId: number, slug: string): void {
		this.#tracked.set(jobId, slug);
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'data.deleted':
				// Every posting is gone, so the list empties at once and the reload confirms it.
				if (this.view.kind === 'loaded') {
					this.postings = [];
					this.hasMore = false;
				}
				void this.load();
				return;
			case 'app.connected':
			case 'app.lagged':
				void this.refresh();
				return;
			case 'job.succeeded': {
				const { job } = event;
				const slug = this.#forget(job.id);
				if (slug !== undefined) {
					const [inserted, updated, removed] = ['inserted', 'updated', 'removed'].map((key) =>
						count(job.result, key)
					);
					toast.success(`${slug}: ${inserted} new, ${updated} updated, ${removed} removed`);
				}
				if (job.kind === SOURCE_BOARD) void this.refresh();
				return;
			}
			case 'run.updated':
				// A new score may change a posting's fit badge.
				if (event.run.workflow === SCORE_FIT_WORKFLOW && event.run.status === 'succeeded') {
					void this.refresh();
				}
				return;
			case 'job.failed': {
				const slug = this.#forget(event.job.id);
				if (slug !== undefined)
					toast.error(`Couldn't source ${slug}. Check the source and the board slug.`);
				return;
			}
		}
	}

	/** Stops a pending filter reload and drops any response in flight. Call when the page goes. */
	destroy(): void {
		this.#cancelDebounce();
		this.#request++;
	}

	#firstPage() {
		return this.#deps.list(this.#deps.client, { ...this.filters }, { limit: PAGE_SIZE });
	}

	#replace(postings: JobPosting[]): void {
		this.postings = postings;
		this.hasMore = postings.length === PAGE_SIZE;
		this.view = { kind: 'loaded' };
	}

	/** The slug of a tracked job, which is no longer tracked afterwards. */
	#forget(jobId: number): string | undefined {
		const slug = this.#tracked.get(jobId);
		this.#tracked.delete(jobId);
		return slug;
	}

	#debounced(): void {
		this.#cancelDebounce();
		this.#debounce = setTimeout(() => void this.load(), FILTER_DEBOUNCE_MS);
	}

	#cancelDebounce(): void {
		clearTimeout(this.#debounce);
		this.#debounce = undefined;
	}
}

export type PostingDetailState =
	| { kind: 'loading' }
	| { kind: 'not-found' }
	| { kind: 'error' }
	| { kind: 'loaded'; posting: JobPostingDetail };

export interface PostingDetailDeps {
	client: ApiClient;
	get: typeof getPosting;
}

/** One posting's page: `GET /postings/{id}`. */
export class PostingDetailStore {
	state = $state<PostingDetailState>({ kind: 'loading' });

	readonly id: number;
	#deps: PostingDetailDeps;

	constructor(id: number, deps: Partial<PostingDetailDeps> = {}) {
		this.id = id;
		this.#deps = {
			client: deps.client ?? createApiClient(),
			get: deps.get ?? getPosting
		};
	}

	/** Loads the posting, showing the skeleton meanwhile. */
	async load(): Promise<void> {
		this.state = { kind: 'loading' };
		const result = await this.#deps.get(this.#deps.client, this.id);
		if (result.kind === 'ok') this.state = { kind: 'loaded', posting: result.data };
		else this.state = { kind: result.kind === 'not-found' ? 'not-found' : 'error' };
	}
}

export type PostingFitState =
	| { kind: 'loading' }
	| { kind: 'error' }
	| { kind: 'loaded'; fit: PostingFit; view: FitView };

export interface PostingFitDeps {
	client: ApiClient;
	get: typeof getPostingFit;
	score: typeof scorePosting;
}

/** The posting a `score_fit` run's input names (`{"posting_id": id}`), or null. */
function scoredPosting(input: unknown): number | null {
	if (typeof input !== 'object' || input === null) return null;
	const id = (input as Record<string, unknown>).posting_id;
	return typeof id === 'number' ? id : null;
}

/**
 * The posting page's Fit section: `GET /postings/{id}/fit`, kept current by `/events`. The
 * skeleton shows until the first load; later reloads keep what's shown, and a response older
 * than the latest request is dropped.
 */
export class PostingFitStore {
	state = $state<PostingFitState>({ kind: 'loading' });
	/** A `POST /postings/{id}/fit` is in flight. */
	starting = $state(false);

	readonly id: number;
	#deps: PostingFitDeps;
	#request = 0;

	constructor(id: number, deps: Partial<PostingFitDeps> = {}) {
		this.id = id;
		this.#deps = {
			client: deps.client ?? createApiClient(),
			get: deps.get ?? getPostingFit,
			score: deps.score ?? scorePosting
		};
	}

	/** Loads the scores. A failure shows the error state until a load succeeds. */
	async load(): Promise<void> {
		const request = ++this.#request;
		const result = await this.#deps.get(this.#deps.client, this.id);
		if (request !== this.#request) return;
		if (result.kind === 'ok') this.#show(result.data);
		else this.state = { kind: 'error' };
	}

	/** Queues a score (or rejoins the waiting one) and shows its run as scoring at once. */
	async score(): Promise<void> {
		if (this.starting) return;
		this.starting = true;
		try {
			const result = await this.#deps.score(this.#deps.client, this.id);
			if (result.kind !== 'ok') {
				toast.error("Couldn't start scoring", {
					description: 'Check that the API is running, then try again.'
				});
				return;
			}
			// Any load in flight predates this run.
			this.#request++;
			const { id, status, error } = result.data.run;
			const run = { id, status, error };
			const fit: PostingFit =
				this.state.kind === 'loaded'
					? { ...this.state.fit, run }
					: { posting_id: this.id, current: null, in_review: null, run };
			this.#show(fit);
		} finally {
			this.starting = false;
		}
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'run.updated':
				if (
					event.run.workflow === SCORE_FIT_WORKFLOW &&
					scoredPosting(event.run.input) === this.id
				) {
					void this.load();
				}
				return;
			case 'app.connected':
			case 'app.lagged':
			case 'data.deleted':
				void this.load();
				return;
		}
	}

	/** Drops any response in flight. Call when the page goes. */
	destroy(): void {
		this.#request++;
	}

	#show(fit: PostingFit): void {
		this.state = { kind: 'loaded', fit, view: fitView(fit) };
	}
}
