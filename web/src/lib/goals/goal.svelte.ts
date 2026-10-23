import { createApiClient, type ApiClient, type Event, type GoalDetail, type PlanEdit } from '#lib/api';
import { toast } from '#lib/components/ui';
import {
	answerGoal,
	editPlan,
	getGoal,
	replanGoal,
	runGoal,
	type Answer,
	type GoalResult
} from './api';

export type GoalState =
	| { kind: 'loading' }
	| { kind: 'not-found' }
	| { kind: 'error' }
	| { kind: 'loaded'; goal: GoalDetail };

export interface GoalDeps {
	client: ApiClient;
	get: typeof getGoal;
	answer: typeof answerGoal;
	edit: typeof editPlan;
	run: typeof runGoal;
	replan: typeof replanGoal;
}

/** The message the editor shows when its base plan is no longer current. */
export const STALE_PLAN = 'This plan changed since you opened it';

const NETWORK = 'Check that the API is running, then try again.';

/**
 * One goal's page: `GET /goals/{id}`, reloaded on `goal.updated` for this goal and after a
 * reconnect or lag, plus its actions (answers, edit, run, replan). The skeleton shows only
 * until the first load; a response older than the latest request is dropped.
 */
export class GoalStore {
	state = $state<GoalState>({ kind: 'loading' });
	/** An action is in flight; its buttons are disabled. */
	busy = $state<'answer' | 'edit' | 'run' | 'replan' | null>(null);

	readonly id: number;
	#deps: GoalDeps;
	#request = 0;

	constructor(id: number, deps: Partial<GoalDeps> = {}) {
		this.id = id;
		this.#deps = {
			client: deps.client ?? createApiClient(),
			get: deps.get ?? getGoal,
			answer: deps.answer ?? answerGoal,
			edit: deps.edit ?? editPlan,
			run: deps.run ?? runGoal,
			replan: deps.replan ?? replanGoal
		};
	}

	/** Loads the goal. A failure keeps a loaded goal on screen and shows the error state otherwise. */
	async load(): Promise<void> {
		const request = ++this.#request;
		const result = await this.#deps.get(this.#deps.client, this.id);
		if (request !== this.#request) return;
		if (result.kind === 'ok') this.state = { kind: 'loaded', goal: result.data };
		else if (result.kind === 'not-found') this.state = { kind: 'not-found' };
		else if (this.state.kind !== 'loaded') this.state = { kind: 'error' };
	}

	/** Submits the answers (blank ones skip their question). */
	answer(answers: Answer[]): Promise<GoalResult<GoalDetail>> {
		return this.#act('answer', () => this.#deps.answer(this.#deps.client, this.id, answers), {
			failed: "Couldn't send the answers"
		});
	}

	/** Runs the current plan. */
	run(): Promise<GoalResult<GoalDetail>> {
		return this.#act('run', () => this.#deps.run(this.#deps.client, this.id), {
			failed: "Couldn't start the plan",
			ok: 'Running the plan'
		});
	}

	/** Asks the planner for a new version. */
	replan(instruction?: string): Promise<GoalResult<GoalDetail>> {
		return this.#act('replan', () => this.#deps.replan(this.#deps.client, this.id, instruction), {
			failed: "Couldn't ask for a replan",
			ok: 'Replanning…'
		});
	}

	/**
	 * Saves an edited plan. A 409 (the plan changed since the editor opened) reloads the goal;
	 * the editor shows {@link STALE_PLAN} and maps a 422's field errors itself.
	 */
	async edit(body: PlanEdit): Promise<GoalResult<GoalDetail>> {
		if (this.busy) return { kind: 'failed' };
		this.busy = 'edit';
		try {
			const result = await this.#deps.edit(this.#deps.client, this.id, body);
			if (result.kind === 'ok') this.#show(result.data);
			else if (result.kind === 'conflict') void this.load();
			return result;
		} finally {
			this.busy = null;
		}
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'goal.updated':
				if (event.goal.id === this.id) void this.load();
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

	#show(goal: GoalDetail): void {
		// Any load in flight predates this answer.
		this.#request++;
		this.state = { kind: 'loaded', goal };
	}

	async #act(
		name: NonNullable<GoalStore['busy']>,
		send: () => Promise<GoalResult<GoalDetail>>,
		messages: { failed: string; ok?: string }
	): Promise<GoalResult<GoalDetail>> {
		if (this.busy) return { kind: 'failed' };
		this.busy = name;
		try {
			const result = await send();
			switch (result.kind) {
				case 'ok':
					this.#show(result.data);
					if (messages.ok) toast.success(messages.ok);
					break;
				case 'conflict':
					toast.error(result.message);
					void this.load();
					break;
				case 'invalid':
					toast.error(messages.failed, { description: result.errors[0]?.message });
					break;
				case 'not-found':
					this.state = { kind: 'not-found' };
					break;
				default:
					toast.error(messages.failed, { description: NETWORK });
			}
			return result;
		} finally {
			this.busy = null;
		}
	}
}
