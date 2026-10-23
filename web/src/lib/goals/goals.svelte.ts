import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { createApiClient, type ApiClient, type Event, type Goal } from '#lib/api';
import { createGoal, listGoals } from './api';

/** Most characters in a goal, as the API allows. */
export const MAX_GOAL = 1000;

export type GoalsView = { kind: 'loading' } | { kind: 'error' } | { kind: 'loaded' };

export interface GoalsDeps {
	client: ApiClient;
	list: typeof listGoals;
	create: typeof createGoal;
	/** Opens goal `id`'s page; `goto` in the app. */
	navigate: (id: number) => unknown;
}

/** The goal page's path. */
export function goalHref(id: number): string {
	return resolve('/goals/[id]', { id: String(id) });
}

/**
 * The Goals page: the list (`GET /goals`, kept current by `/events`) and the composer, which
 * creates a goal and opens its page. The skeleton shows until the first load.
 */
export class GoalsStore {
	view = $state<GoalsView>({ kind: 'loading' });
	goals = $state.raw<Goal[]>([]);
	/** A `POST /goals` is in flight. */
	creating = $state(false);
	/** The composer's error: a 422 message, or a network failure. */
	error = $state<string | null>(null);

	#deps: GoalsDeps;
	#request = 0;

	constructor(deps: Partial<GoalsDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			list: deps.list ?? listGoals,
			create: deps.create ?? createGoal,
			navigate: deps.navigate ?? ((id: number) => goto(resolve('/goals/[id]', { id: String(id) })))
		};
	}

	/** Loads the list. The skeleton shows only until the first load succeeds. */
	async load(): Promise<void> {
		const request = ++this.#request;
		const result = await this.#deps.list(this.#deps.client);
		if (request !== this.#request) return;
		if (result.kind === 'ok') {
			this.goals = result.data;
			this.view = { kind: 'loaded' };
		} else if (this.view.kind !== 'loaded') {
			this.view = { kind: 'error' };
		}
	}

	/** Sends a goal; on 201 opens its page. Returns whether it was created. */
	async create(text: string): Promise<boolean> {
		if (this.creating || !text.trim()) return false;
		this.creating = true;
		this.error = null;
		try {
			const result = await this.#deps.create(this.#deps.client, text.trim());
			switch (result.kind) {
				case 'ok': {
					// eslint-disable-next-line @typescript-eslint/no-unused-vars
					const { questions, plan, versions, run, ...goal } = result.data;
					this.goals = [goal, ...this.goals.filter((held) => held.id !== goal.id)];
					await this.#deps.navigate(goal.id);
					return true;
				}
				case 'invalid':
					this.error =
						result.errors.find((error) => error.field === 'text')?.message ??
						result.errors[0]?.message ??
						"Couldn't save the goal";
					return false;
				default:
					this.error = "Couldn't save the goal. Check that the API is running.";
					return false;
			}
		} finally {
			this.creating = false;
		}
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'goal.updated':
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
}
