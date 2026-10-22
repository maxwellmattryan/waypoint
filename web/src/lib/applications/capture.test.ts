import { describe, expect, it, vi } from 'vitest';
import {
	connectEvents,
	createApiClient,
	type Capture,
	type EventSourceLike,
	type Job
} from '#lib/api';
import type { JobPhase } from '#lib/dashboard/job.js';
import { captureDescription, isCapture, startCapture, watchJob } from './capture';

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

	emit(type: string, data: object = {}): void {
		const message = new MessageEvent(type, {
			data: JSON.stringify({ type, ...data })
		});
		for (const listener of this.#listeners.get(type) ?? []) listener(message);
	}
}

const fakeEvents: typeof connectEvents = (handlers) =>
	connectEvents(handlers, {
		url: 'http://api.test/events',
		EventSource: FakeEventSource
	});

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function clientAnswering(respond: (request: Request) => Response | Promise<Response>) {
	const fetch = vi.fn<typeof globalThis.fetch>(async (input, init) =>
		respond(input instanceof Request ? input : new Request(input, init))
	);
	return {
		client: createApiClient({ baseUrl: 'http://api.test', fetch }),
		fetch
	};
}

const capture: Capture = {
	method: 'json_ld',
	fields: {
		company: 'Acme',
		title: 'Engineer',
		url: 'https://acme.example/jobs/1',
		location: null,
		comp: null
	}
};

function job(overrides: Partial<Job> = {}): Job {
	return {
		id: 7,
		kind: 'capture_extract',
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

describe('startCapture', () => {
	it('maps each status to a result and sends the URL as typed', async () => {
		const cases: [Response | Error, unknown][] = [
			[json(capture), { kind: 'captured', capture }],
			[json({ job_id: 9 }, 202), { kind: 'queued', jobId: 9 }],
			[
				json({ errors: [{ field: 'url', message: 'Required' }] }, 422),
				{ kind: 'invalid', errors: [{ field: 'url', message: 'Required' }] }
			],
			[
				json(
					{
						reason: 'http_status',
						message: 'The site answered with HTTP 404.'
					},
					502
				),
				{ kind: 'unreadable', message: 'The site answered with HTTP 404.' }
			],
			[json({ method: 'json_ld' }), { kind: 'failed' }],
			[json({}, 500), { kind: 'failed' }],
			[json({}, 403), { kind: 'failed' }],
			[new TypeError('network down'), { kind: 'failed' }]
		];
		for (const [answer, want] of cases) {
			const { client, fetch } = clientAnswering(() => {
				if (answer instanceof Error) throw answer;
				return answer.clone();
			});
			expect(await startCapture(client, ' https://acme.example/jobs/1 ')).toEqual(want);
			const request = fetch.mock.calls[0][0] as Request;
			expect(request.method).toBe('POST');
			expect(new URL(request.url).pathname).toBe('/captures');
			expect(await request.json()).toEqual({
				url: ' https://acme.example/jobs/1 '
			});
		}
	});
});

describe('isCapture', () => {
	it('accepts only the Capture shape', () => {
		expect(isCapture(capture)).toBe(true);
		expect(isCapture({ ...capture, method: 'llm' })).toBe(true);
		const bad: unknown[] = [
			null,
			'capture',
			[],
			{ method: 'scraped', fields: capture.fields },
			{ method: 'json_ld' },
			{ method: 'json_ld', fields: { ...capture.fields, url: null } },
			{ method: 'json_ld', fields: { ...capture.fields, title: 3 } },
			{
				method: 'json_ld',
				fields: { company: 'A', title: 'B', url: 'u', location: null }
			}
		];
		for (const value of bad) expect(isCapture(value), JSON.stringify(value)).toBe(false);
	});
});

describe('captureDescription', () => {
	it('names the method, or says nothing was found', () => {
		const check = 'Check the details, then add the application.';
		const cases: [Capture, string][] = [
			[capture, `Filled from the page's job data. ${check}`],
			[{ ...capture, method: 'greenhouse' }, `Filled from Greenhouse. ${check}`],
			[{ ...capture, method: 'lever' }, `Filled from Lever. ${check}`],
			[{ ...capture, method: 'ashby' }, `Filled from Ashby. ${check}`],
			[{ ...capture, method: 'llm' }, `Filled by Claude from the page text. ${check}`],
			[
				{
					method: 'llm',
					fields: { ...capture.fields, company: null, title: null }
				},
				"Couldn't find the details on that page. Fill them in, then add the application."
			]
		];
		for (const [value, want] of cases) expect(captureDescription(value)).toBe(want);
	});
});

describe('watchJob', () => {
	it('reports its own job, refetches on connect and lag, keeps the newer snapshot, and stops when done', async () => {
		let served = job({
			status: 'running',
			attempts: 1,
			updated_at: '2026-10-22T00:00:01Z'
		});
		const { client, fetch } = clientAnswering(() => json(served));
		const updates: [JobPhase, number, string][] = [];
		watchJob(7, { client, connectEvents: fakeEvents }, (phase, snapshot) =>
			updates.push([phase, snapshot.id, snapshot.updated_at])
		);
		const source = FakeEventSource.last!;

		source.emit('app.connected');
		await vi.waitFor(() => expect(updates).toHaveLength(1));
		expect(updates[0]).toEqual(['running', 7, '2026-10-22T00:00:01Z']);
		expect(new URL((fetch.mock.calls[0][0] as Request).url).pathname).toBe('/jobs/7');

		// Another job's events are ignored; an older snapshot of this one too.
		source.emit('job.succeeded', { job: job({ id: 8, status: 'succeeded' }) });
		source.emit('job.queued', {
			job: job({ updated_at: '2026-10-22T00:00:00Z' })
		});
		expect(updates).toHaveLength(1);

		source.emit('job.deferred', {
			job: job({ updated_at: '2026-10-22T00:00:02Z' })
		});
		expect(updates.at(-1)).toEqual(['deferred', 7, '2026-10-22T00:00:02Z']);

		served = job({
			status: 'running',
			attempts: 1,
			updated_at: '2026-10-22T00:00:03Z'
		});
		source.emit('app.lagged');
		await vi.waitFor(() => expect(updates).toHaveLength(3));
		expect(updates.at(-1)?.[0]).toBe('running');
		expect(fetch).toHaveBeenCalledTimes(2);

		source.emit('job.succeeded', {
			job: job({
				status: 'succeeded',
				updated_at: '2026-10-22T00:00:04Z',
				result: capture
			})
		});
		expect(updates.at(-1)?.[0]).toBe('succeeded');
		expect(source.closed).toBe(true);

		source.emit('job.failed', {
			job: job({ status: 'failed', updated_at: '2026-10-22T00:00:05Z' })
		});
		expect(updates).toHaveLength(4);
	});

	it('stops at failed and when stop is called', () => {
		const { client } = clientAnswering(() => json(job()));
		const failed: JobPhase[] = [];
		watchJob(7, { client, connectEvents: fakeEvents }, (phase) => failed.push(phase));
		const first = FakeEventSource.last!;
		first.emit('job.failed', {
			job: job({ status: 'failed', updated_at: '2026-10-22T00:00:01Z' })
		});
		expect(failed).toEqual(['failed']);
		expect(first.closed).toBe(true);

		const seen: JobPhase[] = [];
		const stop = watchJob(7, { client, connectEvents: fakeEvents }, (phase) => seen.push(phase));
		const second = FakeEventSource.last!;
		stop();
		expect(second.closed).toBe(true);
		second.emit('job.started', {
			job: job({ status: 'running', updated_at: '2026-10-22T00:00:01Z' })
		});
		expect(seen).toEqual([]);
	});
});
