import { describe, expect, it, vi } from 'vitest';
import { createApiClient } from '#lib/api';
import { importLinkedIn, importResume, isSummary, summaryText } from './profile';

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function setup(response: () => Response) {
	const fetch = vi.fn<typeof globalThis.fetch>(async () => response());
	const client = createApiClient({ baseUrl: 'http://api.test', fetch });
	return { client, fetch };
}

async function sent(fetch: ReturnType<typeof setup>['fetch']) {
	expect(fetch).toHaveBeenCalledOnce();
	const request = fetch.mock.calls[0][0];
	if (!(request instanceof Request)) throw new Error('expected a Request');
	return request;
}

const zip = () =>
	new File(['PK\u0003\u0004 fake'], 'My export & co.zip', {
		type: 'application/zip'
	});
const summary = { added: 3, already_known: 1, skipped: 0 };

describe('importLinkedIn', () => {
	it('sends the raw file as application/zip with its filename', async () => {
		const { client, fetch } = setup(() => json(summary));
		const file = zip();
		expect(await importLinkedIn(client, file)).toEqual({
			kind: 'ok',
			data: summary
		});
		const request = await sent(fetch);
		expect(request.method).toBe('POST');
		expect(request.url).toBe(
			'http://api.test/profile/linkedin?filename=My%20export%20%26%20co.zip'
		);
		expect(request.headers.get('content-type')).toBe('application/zip');
		expect(new Uint8Array(await request.arrayBuffer())).toEqual(
			new Uint8Array(await file.arrayBuffer())
		);
	});

	it('maps 413 to a file error, 422 to field errors and anything else to failed', async () => {
		const errors = [{ field: 'file', message: 'Must be a LinkedIn data export (.zip)' }];
		const cases: [() => Response, unknown][] = [
			[
				() => new Response(null, { status: 413 }),
				{
					kind: 'invalid',
					errors: [{ field: 'file', message: 'Must be 20 MiB or smaller' }]
				}
			],
			[() => json({ errors }, 422), { kind: 'invalid', errors }],
			[() => new Response(null, { status: 415 }), { kind: 'failed' }],
			[() => new Response('boom', { status: 500 }), { kind: 'failed' }],
			[
				() => {
					throw new TypeError('Failed to fetch');
				},
				{ kind: 'failed' }
			]
		];
		for (const [response, want] of cases) {
			const { client } = setup(response);
			expect(await importLinkedIn(client, zip())).toEqual(want);
		}
	});
});

describe('importResume', () => {
	it('treats 202 as ok', async () => {
		const queued = { job_id: 9, reused: false };
		const { client, fetch } = setup(() => json(queued, 202));
		expect(await importResume(client, 7)).toEqual({ kind: 'ok', data: queued });
		const request = await sent(fetch);
		expect(request.method).toBe('POST');
		expect(request.url).toBe('http://api.test/resumes/7/profile');
	});

	it('maps 404, 422 and 500', async () => {
		const errors = [{ field: 'text', message: 'This resume has no text to read' }];
		const cases: [() => Response, unknown][] = [
			[() => new Response(null, { status: 404 }), { kind: 'not-found' }],
			[() => json({ errors }, 422), { kind: 'invalid', errors }],
			[() => new Response(null, { status: 500 }), { kind: 'failed' }]
		];
		for (const [response, want] of cases) {
			const { client } = setup(response);
			expect(await importResume(client, 7)).toEqual(want);
		}
	});
});

describe('summaryText', () => {
	it.each([
		[{ added: 12, already_known: 3, skipped: 0 }, 'Added 12 memories, 3 already known'],
		[{ added: 1, already_known: 0, skipped: 0 }, 'Added 1 memory'],
		[{ added: 0, already_known: 14, skipped: 0 }, 'Added 0 memories, 14 already known'],
		[{ added: 2, already_known: 0, skipped: 1 }, 'Added 2 memories, 1 skipped'],
		[{ added: 1, already_known: 1, skipped: 4 }, 'Added 1 memory, 1 already known, 4 skipped']
	])('%o reads %s', (input, want) => {
		expect(summaryText(input)).toBe(want);
	});
});

describe('isSummary', () => {
	it('accepts only the summary shape', () => {
		expect(isSummary(summary)).toBe(true);
		for (const bad of [
			null,
			[],
			'x',
			{ added: 1, already_known: 0 },
			{ ...summary, added: '1' },
			{ ...summary, skipped: -1 }
		]) {
			expect(isSummary(bad)).toBe(false);
		}
	});
});
