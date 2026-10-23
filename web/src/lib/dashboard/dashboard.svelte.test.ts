import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	connectEvents,
	createApiClient,
	startTestPrompt,
	type EventSourceLike,
	type GoogleStatus,
	type Job,
	type KillSwitch
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { Dashboard } from './dashboard.svelte';

/** A stand-in for the browser's EventSource that tests drive by hand. */
class FakeEventSource implements EventSourceLike {
	static last: FakeEventSource | null = null;
	onerror: ((event: globalThis.Event) => void) | null = null;
	closed = false;
	#listeners = new Map<string, ((event: MessageEvent) => void)[]>();

	constructor(readonly url: string) {
		FakeEventSource.last = this;
	}

	addEventListener(type: string, listener: (event: MessageEvent) => void): void {
		this.#listeners.set(type, [...(this.#listeners.get(type) ?? []), listener]);
	}

	close(): void {
		this.closed = true;
	}

	/** Delivers one SSE event, as the API sends it: `event: <type>` plus JSON data. */
	emit(type: string, data: object = {}): void {
		const message = new MessageEvent(type, { data: JSON.stringify({ type, ...data }) });
		for (const listener of this.#listeners.get(type) ?? []) listener(message);
	}

	fail(): void {
		this.onerror?.(new globalThis.Event('error'));
	}
}

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

type Route = (request: Request) => Response | Promise<Response>;

/** A `fetch` stub that answers by path; openapi-fetch always passes a single `Request`. */
function stub(routes: Record<string, Route>) {
	return vi.fn<typeof fetch>(async (input, init) => {
		const request = input instanceof Request ? input : new Request(input, init);
		const path = new URL(request.url).pathname;
		const route = routes[path];
		if (!route) throw new TypeError(`no route for ${path}`);
		return route(request);
	});
}

/** Every request the stub received, as `Request`s. */
function requests(fetch: ReturnType<typeof stub>): Request[] {
	return fetch.mock.calls.map(([input, init]) =>
		input instanceof Request ? input : new Request(input, init)
	);
}

const notConfigured: GoogleStatus = {
	configured: false,
	state: 'disconnected',
	connected_at: null,
	scopes: []
};

const running: KillSwitch = { paused: false, updated_at: '2026-10-22T00:00:00Z' };
const paused: KillSwitch = { paused: true, updated_at: '2026-10-22T00:00:05Z' };

function job(overrides: Partial<Job> = {}): Job {
	return {
		id: 7,
		kind: 'test_prompt',
		status: 'queued',
		attempts: 0,
		max_attempts: 3,
		run_at: '2026-10-22T00:00:00Z',
		last_error: null,
		result: null,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		finished_at: null,
		...overrides
	};
}

function setup(routes: Record<string, Route>, start = startTestPrompt) {
	const fetch = stub({
		'/auth/google': () => json(notConfigured),
		'/kill-switch': () => json(running),
		...routes
	});
	const dashboard = new Dashboard({
		client: createApiClient({ baseUrl: 'http://api.test', fetch }),
		startTestPrompt: (opts) => start({ fetch, baseUrl: 'http://api.test', ...opts }),
		connectEvents: (handlers) =>
			connectEvents(handlers, { url: 'http://api.test/events', EventSource: FakeEventSource })
	});
	dashboard.start();
	const source = FakeEventSource.last!;
	return { dashboard, source, fetch };
}

/** Lets pending fetches and their state updates settle. */
async function settle() {
	for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => {
	vi.restoreAllMocks();
	toast.clear();
});

describe('Dashboard', () => {
	it('health: ok, degraded, unreachable', async () => {
		let answer: () => Response = () => json({ status: 'ok', db: 'ok' });
		const { dashboard, source } = setup({ '/health': () => answer() });
		expect(dashboard.health).toEqual({ kind: 'loading' });

		source.emit('app.connected');
		await settle();
		expect(dashboard.health).toEqual({ kind: 'ok' });

		answer = () => json({ status: 'degraded', db: 'unavailable' }, 503);
		source.emit('app.connected');
		await settle();
		expect(dashboard.health).toEqual({ kind: 'degraded' });

		answer = () => new Response('Bad Gateway', { status: 502 });
		source.emit('app.connected');
		await settle();
		expect(dashboard.health).toEqual({ kind: 'unreachable' });

		answer = () => {
			throw new TypeError('Failed to fetch');
		};
		source.emit('app.connected');
		await settle();
		expect(dashboard.health).toEqual({ kind: 'unreachable' });

		// A stream error shows unreachable until the next successful fetch.
		answer = () => json({ status: 'ok', db: 'ok' });
		source.emit('app.connected');
		await settle();
		source.fail();
		expect(dashboard.health).toEqual({ kind: 'unreachable' });
		source.emit('app.connected');
		await settle();
		expect(dashboard.health).toEqual({ kind: 'ok' });
	});

	it('test prompt follows its job over SSE', async () => {
		let fetched = job();
		const { dashboard, source, fetch } = setup({
			'/health': () => json({ status: 'ok', db: 'ok' }),
			'/dev/jobs/test-prompt': () => json({ job_id: 7 }, 202),
			'/jobs/7': () => json(fetched)
		});
		source.emit('app.connected');
		await settle();

		await dashboard.runTest();
		const post = requests(fetch).find((r) => r.url === 'http://api.test/dev/jobs/test-prompt');
		expect(post?.method).toBe('POST');
		expect(post?.headers.get('X-Waypoint-Dev')).toBe('1');
		// It reads the job once to cover events sent before the id was known.
		expect(dashboard.test).toMatchObject({ kind: 'tracking', jobId: 7, phase: 'queued' });
		expect(dashboard.testing).toBe(true);

		source.emit('job.queued', { job: job() });
		expect(dashboard.test).toMatchObject({ phase: 'queued' });

		const started = job({ status: 'running', updated_at: '2026-10-22T00:00:01Z' });
		source.emit('job.started', { job: started });
		expect(dashboard.test).toMatchObject({ phase: 'running' });

		// Events for other jobs change nothing.
		source.emit('job.failed', {
			job: job({ id: 8, status: 'failed', updated_at: '2026-10-22T00:00:09Z' })
		});
		expect(dashboard.test).toMatchObject({ jobId: 7, phase: 'running' });

		// A stale snapshot (older updated_at) can't roll the job back.
		source.emit('job.queued', { job: job() });
		expect(dashboard.test).toMatchObject({ phase: 'running' });

		// On reconnect it re-reads the job; a newer snapshot from the API wins.
		fetched = job({
			status: 'succeeded',
			attempts: 1,
			result: { text: 'pong' },
			updated_at: '2026-10-22T00:00:02Z',
			finished_at: '2026-10-22T00:00:02Z'
		});
		const jobFetches = () =>
			requests(fetch).filter((r) => r.url === 'http://api.test/jobs/7').length;
		const before = jobFetches();
		source.fail();
		expect(dashboard.testing).toBe(true);
		source.emit('app.connected');
		await settle();
		expect(jobFetches()).toBe(before + 1);
		expect(dashboard.test).toMatchObject({
			kind: 'tracking',
			phase: 'succeeded',
			job: { result: { text: 'pong' } }
		});
		expect(dashboard.testing).toBe(false);

		// The succeeded event with the same updated_at keeps the final state.
		source.emit('job.succeeded', { job: fetched });
		expect(dashboard.test).toMatchObject({ phase: 'succeeded', job: { result: { text: 'pong' } } });
	});

	it('test prompt: retrying, deferred and a paused queue', async () => {
		const { dashboard, source } = setup({
			'/health': () => json({ status: 'ok', db: 'ok' }),
			'/dev/jobs/test-prompt': () => json({ job_id: 7 }, 202),
			'/jobs/7': () => json(job({ status: 'queued', attempts: 1 }))
		});
		await dashboard.runTest();
		// A fetched queued job that already ran is retrying.
		expect(dashboard.test).toMatchObject({ phase: 'retrying' });

		source.emit('job.deferred', { job: job({ updated_at: '2026-10-22T00:00:03Z' }) });
		expect(dashboard.test).toMatchObject({ phase: 'deferred' });

		source.emit('queue.paused', { until: '2026-10-22T00:15:00Z' });
		expect(dashboard.queuePausedUntil).toBe('2026-10-22T00:15:00Z');
		source.emit('queue.resumed');
		expect(dashboard.queuePausedUntil).toBeNull();
	});

	it.each([403, 404])('a %i from the test route needs a debug build', async (status) => {
		const error = vi.spyOn(toast, 'error');
		const { dashboard } = setup({ '/dev/jobs/test-prompt': () => new Response(null, { status }) });
		await dashboard.runTest();
		expect(dashboard.test).toEqual({ kind: 'start-failed', reason: 'debug-build' });
		expect(dashboard.testing).toBe(false);
		expect(error).toHaveBeenCalledOnce();
	});

	it('any other test route failure is generic', async () => {
		const error = vi.spyOn(toast, 'error');
		const { dashboard } = setup({ '/dev/jobs/test-prompt': () => new Response(null, { status: 500 }) });
		await dashboard.runTest();
		expect(dashboard.test).toEqual({ kind: 'start-failed', reason: 'other' });
		expect(error).toHaveBeenCalledWith("Couldn't start the test prompt.");
	});

	it('loads the Google status on connect', async () => {
		let answer: () => Response = () => json(notConfigured);
		const { dashboard, source } = setup({
			'/health': () => json({ status: 'ok', db: 'ok' }),
			'/auth/google': () => answer()
		});
		expect(dashboard.google).toEqual({ kind: 'loading' });
		source.emit('app.connected');
		await settle();
		expect(dashboard.google).toEqual({ kind: 'loaded', status: notConfigured });

		answer = () => new Response(null, { status: 500 });
		source.emit('app.connected');
		await settle();
		expect(dashboard.google).toEqual({ kind: 'error' });
	});

	it('kill switch: load, event, toggle, failure', async () => {
		let answer: (request: Request) => Response | Promise<Response> = () => json(running);
		const { dashboard, source, fetch } = setup({
			'/health': () => json({ status: 'ok', db: 'ok' }),
			'/kill-switch': (request) => answer(request)
		});
		expect(dashboard.killSwitch).toEqual({ kind: 'loading' });
		source.emit('app.connected');
		await settle();
		expect(dashboard.killSwitch).toEqual({ kind: 'loaded', state: running });

		// Another tab (or delete-all) paused agents.
		source.emit('kill_switch.updated', { kill_switch: paused });
		expect(dashboard.killSwitch).toEqual({ kind: 'loaded', state: paused });

		// Resume: a PUT with the new value; the response is applied, saving meanwhile.
		answer = () => json(running);
		const resuming = dashboard.setPaused(false);
		expect(dashboard.savingKillSwitch).toBe(true);
		await resuming;
		expect(dashboard.savingKillSwitch).toBe(false);
		expect(dashboard.killSwitch).toEqual({ kind: 'loaded', state: running });
		const put = requests(fetch).find((r) => r.method === 'PUT');
		expect(put?.url).toBe('http://api.test/kill-switch');
		expect(await put?.json()).toEqual({ paused: false });

		// A failed change shows the error toast and keeps the state.
		const error = vi.spyOn(toast, 'error');
		answer = (request) =>
			request.method === 'PUT' ? new Response(null, { status: 500 }) : json(running);
		await dashboard.setPaused(true);
		expect(error).toHaveBeenCalledWith("Couldn't change the kill switch.");
		expect(dashboard.killSwitch).toEqual({ kind: 'loaded', state: running });
		expect(dashboard.savingKillSwitch).toBe(false);

		// A failed load shows the error state.
		answer = () => new Response(null, { status: 500 });
		source.emit('app.connected');
		await settle();
		expect(dashboard.killSwitch).toEqual({ kind: 'error' });
	});

	it('delete all data: success, not_revoked warning, failure', async () => {
		let google: 'revoked' | 'not_revoked' = 'revoked';
		let deleteStatus = 200;
		let killSwitch = running;
		const { dashboard, source, fetch } = setup({
			'/health': () => json({ status: 'ok', db: 'ok' }),
			'/kill-switch': () => json(killSwitch),
			'/data': () =>
				deleteStatus === 200 ? json({ google }) : new Response(null, { status: deleteStatus })
		});
		source.emit('app.connected');
		await settle();
		const error = vi.spyOn(toast, 'error');

		expect(await dashboard.deleteAllData('delete all data')).toEqual({ google: 'revoked' });
		const request = requests(fetch).find((r) => r.method === 'DELETE');
		expect(request?.url).toBe('http://api.test/data');
		expect(await request?.json()).toEqual({ confirmation: 'delete all data' });

		// The warning about the grant is the caller's to show; the store only passes it on.
		google = 'not_revoked';
		expect(await dashboard.deleteAllData('delete all data')).toEqual({ google: 'not_revoked' });
		expect(error).not.toHaveBeenCalled();

		deleteStatus = 422;
		expect(await dashboard.deleteAllData('nope')).toBeNull();
		expect(error).toHaveBeenCalledWith("Couldn't delete the data.");

		// data.deleted reloads Google and the kill switch.
		const loads = (path: string) =>
			requests(fetch).filter((r) => r.method === 'GET' && r.url === `http://api.test${path}`)
				.length;
		const before = [loads('/auth/google'), loads('/kill-switch')];
		killSwitch = paused;
		source.emit('data.deleted');
		await settle();
		expect([loads('/auth/google'), loads('/kill-switch')]).toEqual([before[0] + 1, before[1] + 1]);
		expect(dashboard.killSwitch).toEqual({ kind: 'loaded', state: paused });
	});

	it('forwards every event to subscribers and ignores application events', async () => {
		const { dashboard, source, fetch } = setup({
			'/health': () => json({ status: 'ok', db: 'ok' }),
			'/dev/jobs/test-prompt': () => json({ job_id: 7 }, 202),
			'/jobs/7': () => json(job())
		});
		source.emit('app.connected');
		await settle();
		await dashboard.runTest();
		const state = () =>
			JSON.parse(
				JSON.stringify({
					connection: dashboard.connection,
					health: dashboard.health,
					google: dashboard.google,
					test: dashboard.test,
					queuePausedUntil: dashboard.queuePausedUntil
				})
			);
		const before = state();
		const fetches = fetch.mock.calls.length;

		const seen: string[] = [];
		const unsubscribe = dashboard.subscribe((event) => seen.push(event.type));
		// Same id and a later updated_at than the tracked job, so a misrouted event would show.
		const application = {
			id: 7,
			company: 'Acme',
			title: 'Engineer',
			url: null,
			location: null,
			comp: null,
			source: null,
			notes: null,
			status: 'applied',
			status_changed_at: '2026-10-22T00:00:09Z',
			created_at: '2026-10-22T00:00:00Z',
			updated_at: '2026-10-22T00:00:09Z'
		};
		source.emit('application.created', { application });
		source.emit('application.updated', { application });
		source.emit('application.deleted', { id: 7 });
		await settle();
		expect(state()).toEqual(before);
		expect(fetch.mock.calls.length).toBe(fetches);

		source.emit('app.lagged', { skipped: 2 });
		source.emit('app.connected');
		expect(seen).toEqual([
			'application.created',
			'application.updated',
			'application.deleted',
			'app.lagged',
			'app.connected'
		]);

		unsubscribe();
		source.emit('application.deleted', { id: 7 });
		expect(seen).toHaveLength(5);
		await settle();
	});

	it('connection state', () => {
		const { dashboard, source } = setup({ '/health': () => json({ status: 'ok', db: 'ok' }) });
		expect(dashboard.connection).toBe('connecting');
		source.emit('app.connected');
		expect(dashboard.connection).toBe('live');
		source.fail();
		expect(dashboard.connection).toBe('reconnecting');
		source.emit('app.connected');
		expect(dashboard.connection).toBe('live');

		dashboard.stop();
		expect(source.closed).toBe(true);
	});
});
