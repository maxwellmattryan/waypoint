import { createApiClient, type ApiClient, type Memory, type MemoryStatus } from '#lib/api';
import { normaliseTag, toast } from '#lib/components/ui';
import { confirmMemory, deleteMemory, listMemories } from './api';

/** How long search waits after the last keystroke before it reloads. */
export const SEARCH_DEBOUNCE_MS = 300;

/** The page's filters. Blank means "any". */
export interface Filters {
	q: string;
	status: MemoryStatus | '';
	tag: string;
}

/** Whether the list has loaded. `loading` shows the skeleton; later reloads keep the list. */
export type ListView = { kind: 'loading' } | { kind: 'error' } | { kind: 'loaded' };

/** Whether any filter is set. */
export function hasFilters(filters: Filters): boolean {
	return filters.q.trim() !== '' || filters.status !== '' || filters.tag.trim() !== '';
}

/**
 * Whether `memory` can be put in the list in place under `filters`. Search ranks on the
 * server, so any active search means a reload instead.
 */
export function matches(memory: Memory, filters: Filters): boolean {
	if (filters.q.trim() !== '') return false;
	if (filters.status && memory.status !== filters.status) return false;
	const tag = normaliseTag(filters.tag);
	return !tag || memory.tags.includes(tag);
}

export interface MemoryListDeps {
	client: ApiClient;
	list: typeof listMemories;
	confirm: typeof confirmMemory;
	remove: typeof deleteMemory;
}

/**
 * The memories page's list: loads with the current filters, reloads when they change (search
 * after a 300ms pause), drops responses for older filters, and applies the page's own changes
 * in place. Owns the confirm, delete and save toasts.
 */
export class MemoryList {
	filters = $state<Filters>({ q: '', status: '', tag: '' });
	view = $state<ListView>({ kind: 'loading' });
	/** The memories held, in the API's order. */
	memories = $state.raw<Memory[]>([]);

	#deps: MemoryListDeps;
	/** Bumped per request; a response for an older one is dropped. */
	#request = 0;
	#debounce: ReturnType<typeof setTimeout> | undefined;

	constructor(deps: Partial<MemoryListDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			list: deps.list ?? listMemories,
			confirm: deps.confirm ?? confirmMemory,
			remove: deps.remove ?? deleteMemory
		};
	}

	get filtered(): boolean {
		return hasFilters(this.filters);
	}

	/** Loads with the current filters. The skeleton shows only until the first load succeeds. */
	async load(): Promise<void> {
		clearTimeout(this.#debounce);
		this.#debounce = undefined;
		const request = ++this.#request;
		if (this.view.kind !== 'loaded') this.view = { kind: 'loading' };
		const result = await this.#deps.list(this.#deps.client, { ...this.filters });
		if (request !== this.#request) return;
		if (result.kind === 'ok') {
			this.memories = result.data;
			this.view = { kind: 'loaded' };
		} else {
			this.view = { kind: 'error' };
		}
	}

	/** Sets the search text; reloads 300ms after the last call. */
	setQuery(q: string): void {
		this.filters.q = q;
		clearTimeout(this.#debounce);
		this.#debounce = setTimeout(() => void this.load(), SEARCH_DEBOUNCE_MS);
	}

	setStatus(status: MemoryStatus | ''): Promise<void> {
		this.filters.status = status;
		return this.load();
	}

	/** Filters by one tag, normalised; `''` clears the tag filter. */
	setTag(tag: string): Promise<void> {
		this.filters.tag = normaliseTag(tag);
		return this.load();
	}

	clearFilters(): Promise<void> {
		this.filters = { q: '', status: '', tag: '' };
		return this.load();
	}

	/** Applies a memory the dialog saved: in place when it still matches the filters, else a reload. */
	saved(memory: Memory, wasNew: boolean): void {
		toast.success('Memory saved');
		this.#apply(memory, wasNew);
	}

	#apply(memory: Memory, wasNew: boolean): void {
		if (this.view.kind !== 'loaded' || !matches(memory, this.filters)) {
			void this.load();
			return;
		}
		const held = this.memories.some((m) => m.id === memory.id);
		if (wasNew || !held) {
			// The newest update sorts first.
			this.memories = [memory, ...this.memories.filter((m) => m.id !== memory.id)];
		} else {
			this.memories = this.memories.map((m) => (m.id === memory.id ? memory : m));
		}
	}

	/** Takes a memory out of the list. */
	remove(id: number): void {
		this.memories = this.memories.filter((m) => m.id !== id);
	}

	/** Confirms a proposed memory and updates it in the list. */
	async confirm(id: number): Promise<boolean> {
		const result = await this.#deps.confirm(this.#deps.client, id);
		switch (result.kind) {
			case 'ok':
				toast.success('Memory confirmed');
				this.#apply(result.data, false);
				return true;
			case 'not-found':
				toast.error("Couldn't confirm the memory", {
					description: 'It was deleted elsewhere.'
				});
				this.remove(id);
				return false;
			default:
				toast.error("Couldn't confirm the memory");
				return false;
		}
	}

	/**
	 * Deletes a memory. It leaves the list on success and on a 404 (already gone); any other
	 * failure keeps it. Resolves whether the confirm dialog should close.
	 */
	async delete(id: number): Promise<boolean> {
		const result = await this.#deps.remove(this.#deps.client, id);
		if (result.kind === 'ok' || result.kind === 'not-found') {
			this.remove(id);
			toast.success('Memory deleted');
			return true;
		}
		toast.error("Couldn't delete the memory");
		return false;
	}

	/** Stops a pending search reload. Call when the page goes away. */
	destroy(): void {
		clearTimeout(this.#debounce);
		this.#request++;
	}
}
