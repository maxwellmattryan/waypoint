import { getContext, setContext } from 'svelte';
import {
	connectEvents as realConnectEvents,
	createApiClient,
	startTestPrompt as realStartTestPrompt,
	type ApiClient,
	type DataDeletion,
	type Event,
	type GoogleStatus,
	type Job,
	type KillSwitch
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { inFlight, newerJob, phaseOfEvent, phaseOfJob, type JobPhase } from './job';

/** The live-connection state of the `/events` stream. */
export type ConnectionState = 'connecting' | 'live' | 'reconnecting';

/** API and database health, from `GET /health`. */
export type HealthState =
	| { kind: 'loading' }
	| { kind: 'ok' }
	| { kind: 'degraded' }
	| { kind: 'unreachable' };

/** The Google connection, from `GET /auth/google`. */
export type GoogleState =
	| { kind: 'loading' }
	| { kind: 'error' }
	| { kind: 'loaded'; status: GoogleStatus };

/** The kill switch, from `GET /kill-switch`. */
export type KillSwitchState =
	| { kind: 'loading' }
	| { kind: 'error' }
	| { kind: 'loaded'; state: KillSwitch };

/** Why starting a test prompt failed. */
export type StartFailure = 'debug-build' | 'other';

/** The "Test Claude" flow. */
export type TestState =
	| { kind: 'idle' }
	| { kind: 'starting' }
	| { kind: 'start-failed'; reason: StartFailure }
	| { kind: 'tracking'; jobId: number; job: Job | null; phase: JobPhase };

export interface DashboardDeps {
	client: ApiClient;
	startTestPrompt: typeof realStartTestPrompt;
	connectEvents: typeof realConnectEvents;
}

/**
 * The dashboard's state and data flow: one instance per app, shared by the shell's
 * connection badge and the dashboard page. `start()` opens the event stream; every
 * `app.connected` (re)loads health, Google, the kill switch and the tracked job, since
 * there is no replay.
 */
export class Dashboard {
	connection = $state<ConnectionState>('connecting');
	health = $state<HealthState>({ kind: 'loading' });
	google = $state<GoogleState>({ kind: 'loading' });
	killSwitch = $state<KillSwitchState>({ kind: 'loading' });
	/** Whether a kill-switch change is in flight. */
	savingKillSwitch = $state(false);
	test = $state<TestState>({ kind: 'idle' });
	/** When the job queue resumes after a usage limit; `null` while it isn't paused. */
	queuePausedUntil = $state<string | null>(null);

	/** Whether a test prompt is being started or its job is still in flight. */
	testing = $derived(
		this.test.kind === 'starting' || (this.test.kind === 'tracking' && inFlight(this.test.phase))
	);

	#deps: DashboardDeps;
	#close: (() => void) | null = null;
	#listeners: ((event: Event) => void)[] = [];
	/** Bumped on every stream error so a health fetch started before it can't report OK. */
	#healthGeneration = 0;

	constructor(deps: Partial<DashboardDeps> = {}) {
		this.#deps = {
			client: deps.client ?? createApiClient(),
			startTestPrompt: deps.startTestPrompt ?? realStartTestPrompt,
			connectEvents: deps.connectEvents ?? realConnectEvents
		};
	}

	/** Opens the event stream. Data loads on its first `app.connected`. */
	start(): void {
		if (this.#close) return;
		this.#close = this.#deps.connectEvents({
			event: (event) => this.#onEvent(event),
			error: () => this.#onError()
		});
	}

	/** Closes the event stream. */
	stop(): void {
		this.#close?.();
		this.#close = null;
	}

	/** Starts a `test_prompt` job and tracks it. */
	async runTest(): Promise<void> {
		if (this.testing) return;
		this.test = { kind: 'starting' };
		const started = await this.#deps.startTestPrompt();
		if (!started.ok) {
			const reason: StartFailure =
				started.status === 403 || started.status === 404 ? 'debug-build' : 'other';
			this.test = { kind: 'start-failed', reason };
			toast.error(
				reason === 'debug-build'
					? 'The test route needs a debug API build.'
					: "Couldn't start the test prompt."
			);
			return;
		}
		this.test = { kind: 'tracking', jobId: started.jobId, job: null, phase: 'queued' };
		// Events published before the id was known were ignored; read the current state once.
		await this.#refreshJob();
	}

	/** Pauses or resumes every agent. A failure shows an error toast and changes nothing. */
	async setPaused(paused: boolean): Promise<void> {
		if (this.savingKillSwitch) return;
		this.savingKillSwitch = true;
		try {
			const { data } = await this.#deps.client.PUT('/kill-switch', { body: { paused } });
			if (data) this.killSwitch = { kind: 'loaded', state: data };
			else toast.error("Couldn't change the kill switch.");
		} catch {
			toast.error("Couldn't change the kill switch.");
		} finally {
			this.savingKillSwitch = false;
		}
	}

	/**
	 * Deletes all data (`DELETE /data`). Resolves the body on success; on failure shows an
	 * error toast and resolves `null`. The success toasts are the caller's.
	 */
	async deleteAllData(confirmation: string): Promise<DataDeletion | null> {
		try {
			const { data } = await this.#deps.client.DELETE('/data', { body: { confirmation } });
			if (data) return data;
		} catch {
			// Shown below.
		}
		toast.error("Couldn't delete the data.");
		return null;
	}

	/**
	 * Calls `listener` with every event from the stream, `app.connected` and `app.lagged`
	 * included, until the returned function is called. Pages use this to share the app's
	 * one `EventSource`.
	 */
	subscribe(listener: (event: Event) => void): () => void {
		this.#listeners = [...this.#listeners, listener];
		return () => {
			this.#listeners = this.#listeners.filter((held) => held !== listener);
		};
	}

	#onEvent(event: Event): void {
		for (const listener of this.#listeners) listener(event);
		switch (event.type) {
			case 'app.connected':
				this.connection = 'live';
				void this.#loadHealth();
				void this.#loadGoogle();
				void this.#loadKillSwitch();
				void this.#refreshJob();
				return;
			case 'app.lagged':
				void this.#refreshJob();
				return;
			case 'kill_switch.updated':
				this.killSwitch = { kind: 'loaded', state: event.kill_switch };
				return;
			case 'data.deleted':
				void this.#loadGoogle();
				void this.#loadKillSwitch();
				return;
			case 'queue.paused':
				this.queuePausedUntil = event.until;
				return;
			case 'queue.resumed':
				this.queuePausedUntil = null;
				return;
			// Subscribers such as the applications and runs stores handle these.
			case 'application.created':
			case 'application.updated':
			case 'application.deleted':
			case 'run.updated':
				return;
			default:
				this.#applyJob(event.job, phaseOfEvent(event.type), true);
		}
	}

	#onError(): void {
		this.connection = 'reconnecting';
		this.#healthGeneration++;
		this.health = { kind: 'unreachable' };
	}

	/**
	 * Applies a snapshot of the tracked job. Snapshots of other jobs are ignored, and of two
	 * snapshots the later `updated_at` wins; on a tie an event beats a fetch.
	 */
	#applyJob(job: Job, phase: JobPhase, fromEvent: boolean): void {
		const test = this.test;
		if (test.kind !== 'tracking' || job.id !== test.jobId) return;
		if (test.job) {
			const winner = fromEvent ? newerJob(job, test.job) : newerJob(test.job, job);
			if (winner !== job) return;
		}
		this.test = { kind: 'tracking', jobId: test.jobId, job, phase };
	}

	async #refreshJob(): Promise<void> {
		if (this.test.kind !== 'tracking') return;
		const id = this.test.jobId;
		try {
			const { data } = await this.#deps.client.GET('/jobs/{id}', { params: { path: { id } } });
			if (data) this.#applyJob(data, phaseOfJob(data), false);
		} catch {
			// The stream reconnects and refreshes again; nothing to show meanwhile.
		}
	}

	async #loadHealth(): Promise<void> {
		const generation = this.#healthGeneration;
		let next: HealthState;
		try {
			const { response } = await this.#deps.client.GET('/health');
			next =
				response.status === 200
					? { kind: 'ok' }
					: response.status === 503
						? { kind: 'degraded' }
						: { kind: 'unreachable' };
		} catch {
			next = { kind: 'unreachable' };
		}
		if (generation === this.#healthGeneration) this.health = next;
	}

	async #loadKillSwitch(): Promise<void> {
		try {
			const { data } = await this.#deps.client.GET('/kill-switch');
			this.killSwitch = data ? { kind: 'loaded', state: data } : { kind: 'error' };
		} catch {
			this.killSwitch = { kind: 'error' };
		}
	}

	async #loadGoogle(): Promise<void> {
		try {
			const { data } = await this.#deps.client.GET('/auth/google');
			this.google = data ? { kind: 'loaded', status: data } : { kind: 'error' };
		} catch {
			this.google = { kind: 'error' };
		}
	}
}

const KEY = Symbol('dashboard');

/** Makes `dashboard` available to descendant components. */
export function setDashboard(dashboard: Dashboard): Dashboard {
	return setContext(KEY, dashboard);
}

/** The {@link Dashboard} that the root layout provides. */
export function getDashboard(): Dashboard {
	const dashboard = getContext<Dashboard | undefined>(KEY);
	if (!dashboard) throw new Error('getDashboard() needs the root layout to call setDashboard()');
	return dashboard;
}
