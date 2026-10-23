import type { ApiClient, Goal, GoalDetail, PlanEdit } from '#lib/api';
import { run, type Result, type Sent } from '#lib/applications/api.js';

export type { Result };

/**
 * What a goal route came to: a {@link Result} whose `conflict` (409) carries the API's message,
 * e.g. "This plan needs review before it can run".
 */
export type GoalResult<T> =
	| Exclude<Result<T>, { kind: 'conflict' }>
	| { kind: 'conflict'; message: string };

/** The message of a 409 whose body didn't parse. */
export const CONFLICT_FALLBACK = 'Nothing changed: this goal changed since the page loaded.';

function conflictMessage(error: unknown): string {
	if (typeof error === 'object' && error !== null && 'message' in error) {
		const { message } = error;
		if (typeof message === 'string' && message.trim()) return message;
	}
	return CONFLICT_FALLBACK;
}

/** {@link run}, keeping the 409 body's message. */
async function send<T>(ok: number, request: () => Promise<Sent<T>>): Promise<GoalResult<T>> {
	let body: unknown;
	const result = await run(ok, async () => {
		const sent = await request();
		body = sent.error;
		return sent;
	});
	return result.kind === 'conflict' ? { kind: 'conflict', message: conflictMessage(body) } : result;
}

const path = (id: number) => ({ path: { id } });

/** Up to 200 goals, newest first. */
export function listGoals(client: ApiClient): Promise<Result<Goal[]>> {
	return run(200, () => client.GET('/goals'));
}

/** One goal with its questions, plan, versions and latest run; a 404 is `not-found`. */
export function getGoal(client: ApiClient, id: number): Promise<Result<GoalDetail>> {
	return run(200, () => client.GET('/goals/{id}', { params: path(id) }));
}

/** Stores a goal and queues its planner; a 422 is `invalid` with the field errors. */
export function createGoal(client: ApiClient, text: string): Promise<Result<GoalDetail>> {
	return run(201, () => client.POST('/goals', { body: { text } }));
}

/** One answer; a blank or missing `answer` skips the question. */
export interface Answer {
	question_id: number;
	answer?: string;
}

/** Answers the open questions and plans again. */
export function answerGoal(
	client: ApiClient,
	id: number,
	answers: Answer[]
): Promise<GoalResult<GoalDetail>> {
	return send(202, () =>
		client.POST('/goals/{id}/answers', { params: path(id), body: { answers } })
	);
}

/** Replaces the plan with the user's own, as a new version. */
export function editPlan(
	client: ApiClient,
	id: number,
	body: PlanEdit
): Promise<GoalResult<GoalDetail>> {
	return send(200, () => client.PUT('/goals/{id}/plan', { params: path(id), body }));
}

/** Runs the current plan. */
export function runGoal(client: ApiClient, id: number): Promise<GoalResult<GoalDetail>> {
	return send(202, () => client.POST('/goals/{id}/run', { params: path(id), body: {} }));
}

/** Asks the planner for a new version, with an optional instruction. */
export function replanGoal(
	client: ApiClient,
	id: number,
	instruction?: string
): Promise<GoalResult<GoalDetail>> {
	const body = instruction?.trim() ? { instruction: instruction.trim() } : {};
	return send(202, () => client.POST('/goals/{id}/replan', { params: path(id), body }));
}
