import type {
	ApiClient,
	Memory,
	MemoryInput,
	MemoryKind,
	MemorySignal,
	MemoryStatus
} from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';

export type { Result };

/** `GET /memories` filters. A blank or missing value means "any". */
export interface MemoryFilters {
	kind?: MemoryKind | '';
	status?: MemoryStatus | '';
	tag?: string;
	/** Full-text search. */
	q?: string;
}

const path = (id: number) => ({ params: { path: { id } } });

/** The query for `filters`: only the non-blank values, trimmed. */
export function memoryQuery(filters: MemoryFilters): {
	kind?: MemoryKind;
	status?: MemoryStatus;
	tag?: string;
	q?: string;
} {
	const query: { kind?: MemoryKind; status?: MemoryStatus; tag?: string; q?: string } = {};
	if (filters.kind) query.kind = filters.kind;
	if (filters.status) query.status = filters.status;
	const tag = filters.tag?.trim();
	if (tag) query.tag = tag;
	const q = filters.q?.trim();
	if (q) query.q = q;
	return query;
}

/** The memories matching `filters`: best match first with `q`, else most recently updated first. */
export function listMemories(
	client: ApiClient,
	filters: MemoryFilters = {}
): Promise<Result<Memory[]>> {
	return run(200, () => client.GET('/memories', { params: { query: memoryQuery(filters) } }));
}

export function getMemory(client: ApiClient, id: number): Promise<Result<Memory>> {
	return run(200, () => client.GET('/memories/{id}', path(id)));
}

/** The learning signals behind memory `id`, newest first; `[]` when it has none. */
export function listMemoryEvidence(
	client: ApiClient,
	id: number
): Promise<Result<MemorySignal[]>> {
	return run(200, () => client.GET('/memories/{id}/evidence', path(id)));
}

/** Adds a memory; the API stores it confirmed, with source `manual`. */
export function createMemory(client: ApiClient, body: MemoryInput): Promise<Result<Memory>> {
	return run(201, () => client.POST('/memories', { body }));
}

/** Replaces a memory's kind, topic, content and tags; its status and source are kept. */
export function updateMemory(
	client: ApiClient,
	id: number,
	body: MemoryInput
): Promise<Result<Memory>> {
	return run(200, () => client.PUT('/memories/{id}', { ...path(id), body }));
}

/** Confirms a proposed memory; an already confirmed one comes back unchanged. */
export function confirmMemory(client: ApiClient, id: number): Promise<Result<Memory>> {
	return run(200, () => client.POST('/memories/{id}/confirm', path(id)));
}

export function deleteMemory(client: ApiClient, id: number): Promise<Result<void>> {
	return run<void>(204, () => client.DELETE('/memories/{id}', path(id)));
}
