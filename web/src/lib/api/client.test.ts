import { describe, expect, it, vi } from 'vitest';
import { apiBaseUrl } from '#lib/config.js';
import { createApiClient, type Event, type Job } from '#lib/api';

const job: Job = {
	id: 7,
	kind: 'test_prompt',
	status: 'succeeded',
	attempts: 1,
	max_attempts: 3,
	run_at: '2026-10-22T00:00:00Z',
	last_error: null,
	result: { text: 'hi' },
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:01Z',
	finished_at: '2026-10-22T00:00:01Z'
};

/** A `fetch` stub. openapi-fetch always calls it with a single `Request`. */
function stub(response: () => Response) {
	return vi.fn<typeof fetch>(async () => response());
}

function requestOf(fetch: ReturnType<typeof stub>, call = 0): Request {
	const input = fetch.mock.calls[call][0];
	if (!(input instanceof Request)) throw new Error('expected a Request');
	return input;
}

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

describe('createApiClient', () => {
	it('builds the job URL and returns data', async () => {
		const fetch = stub(() => json(job));
		const client = createApiClient({ baseUrl: 'http://api.test', fetch });

		const { data, error } = await client.GET('/jobs/{id}', { params: { path: { id: 7 } } });

		expect(fetch).toHaveBeenCalledOnce();
		const request = requestOf(fetch);
		expect(request.method).toBe('GET');
		expect(request.url).toBe('http://api.test/jobs/7');
		expect(data).toEqual(job);
		expect(error).toBeUndefined();
	});

	it('returns error on 404', async () => {
		const fetch = stub(() => new Response(null, { status: 404 }));
		const client = createApiClient({ baseUrl: 'http://api.test', fetch });

		const { data, error, response } = await client.GET('/jobs/{id}', {
			params: { path: { id: 404 } }
		});

		expect(response.status).toBe(404);
		expect(data).toBeUndefined();
		expect(error).toBeDefined();
	});

	it('defaults to apiBaseUrl()', async () => {
		const fetch = stub(() => json({ status: 'ok', db: 'ok' }));
		const client = createApiClient({ fetch });

		const { data } = await client.GET('/health');

		expect(requestOf(fetch).url).toBe(`${apiBaseUrl()}/health`);
		expect(data).toEqual({ status: 'ok', db: 'ok' });
	});

	it('is typed by the generated schema', () => {
		const client = createApiClient({ baseUrl: 'http://api.test', fetch: stub(() => json(job)) });
		// Type-level checks: `npm run check` fails if any of these stop being errors.
		const typeChecks = () => {
			// @ts-expect-error unknown path
			void client.GET('/nope');
			// @ts-expect-error missing path param
			void client.GET('/jobs/{id}', { params: { path: {} } });
		};

		const event = { type: 'job.failed', job } as Event;
		let status: string | undefined;
		if (event.type === 'job.failed') {
			status = event.job.status;
		}

		expect(typeof typeChecks).toBe('function');
		expect(status).toBe('succeeded');
	});
});
