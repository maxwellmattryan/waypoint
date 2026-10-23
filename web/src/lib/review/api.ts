import type { AgentAction, ApiClient, PendingAction, ReviewItem } from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';
import { PAGE_SIZE } from '#lib/runs/api.js';

export type { Result };

const path = (id: number) => ({ params: { path: { id } } });

/** The actions waiting at the approval gate, oldest first. */
export function listPendingActions(client: ApiClient): Promise<Result<PendingAction[]>> {
	return run(200, () => client.GET('/pending-actions'));
}

/** The results an agent wasn't sure of, oldest first. */
export function listReviewItems(client: ApiClient): Promise<Result<ReviewItem[]>> {
	return run(200, () => client.GET('/review-items'));
}

/**
 * Approves a pending action: the requested input when `input` is undefined, else the edited
 * one. A 422 (the edited input breaks the tool's schema) is `invalid`.
 */
export function approveAction(
	client: ApiClient,
	id: number,
	input?: Record<string, unknown>
): Promise<Result<PendingAction>> {
	const body = input === undefined ? {} : { input };
	return run(200, () => client.POST('/pending-actions/{id}/approve', { ...path(id), body }));
}

/** Rejects a pending action, which ends its run `rejected`. */
export function rejectAction(client: ApiClient, id: number): Promise<Result<PendingAction>> {
	return run(200, () => client.POST('/pending-actions/{id}/reject', path(id)));
}

/** Clears a review item from the queue without applying it. */
export function dismissReviewItem(client: ApiClient, id: number): Promise<Result<ReviewItem>> {
	return run(200, () => client.POST('/review-items/{id}/dismiss', path(id)));
}

export interface ListActionsOptions {
	limit?: number;
	/** Only actions with an id below this one: the next page. */
	before?: number;
}

/** The action log, newest first, a page at a time. */
export function listActions(
	client: ApiClient,
	{ limit = PAGE_SIZE, before }: ListActionsOptions = {}
): Promise<Result<AgentAction[]>> {
	const query = before === undefined ? { limit } : { limit, before };
	return run(200, () => client.GET('/actions', { params: { query } }));
}

/** Reverts one logged action; resolves to it with `undone_at` set. */
export function undoAction(client: ApiClient, id: number): Promise<Result<AgentAction>> {
	return run(200, () => client.POST('/actions/{id}/undo', path(id)));
}
