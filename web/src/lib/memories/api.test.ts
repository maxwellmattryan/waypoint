import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type Memory, type MemoryInput, type MemorySignal } from '#lib/api';
import {
	confirmMemory,
	createMemory,
	deleteMemory,
	getMemory,
	listMemories,
	listMemoryEvidence,
	updateMemory,
	type Result
} from './api';

const memory: Memory = {
	id: 7,
	kind: 'preference',
	topic: 'location',
	content: 'Remote only',
	tags: ['remote'],
	status: 'confirmed',
	confirmed_at: '2026-10-22T00:00:00Z',
	source_kind: 'manual',
	source_ref: null,
	source_label: null,
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:00Z'
};
const signal: MemorySignal = {
	id: 3,
	kind: 'rejection',
	run_id: 12,
	pending_action_id: 5,
	review_item_id: null,
	subject: 'send_email',
	drafted: { to: 'a@example.test' },
	approved: null,
	changes: null,
	reason: null,
	created_at: '2026-10-22T00:00:00Z'
};
const input: MemoryInput = {
	kind: 'preference',
	topic: 'location',
	content: 'Remote only',
	tags: ['remote']
};

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

type Call = (client: ReturnType<typeof setup>['client']) => Promise<Result<unknown>>;

const calls: {
	name: string;
	call: Call;
	ok: () => Response;
	data: unknown;
	method: string;
	url: string;
	body?: unknown;
}[] = [
	{
		name: 'list',
		call: (c) => listMemories(c),
		ok: () => json([memory]),
		data: [memory],
		method: 'GET',
		url: 'http://api.test/memories'
	},
	{
		name: 'get',
		call: (c) => getMemory(c, 7),
		ok: () => json(memory),
		data: memory,
		method: 'GET',
		url: 'http://api.test/memories/7'
	},
	{
		name: 'evidence',
		call: (c) => listMemoryEvidence(c, 7),
		ok: () => json([signal]),
		data: [signal],
		method: 'GET',
		url: 'http://api.test/memories/7/evidence'
	},
	{
		name: 'create',
		call: (c) => createMemory(c, input),
		ok: () => json(memory, 201),
		data: memory,
		method: 'POST',
		url: 'http://api.test/memories',
		body: input
	},
	{
		name: 'update',
		call: (c) => updateMemory(c, 7, input),
		ok: () => json(memory),
		data: memory,
		method: 'PUT',
		url: 'http://api.test/memories/7',
		body: input
	},
	{
		name: 'confirm',
		call: (c) => confirmMemory(c, 7),
		ok: () => json(memory),
		data: memory,
		method: 'POST',
		url: 'http://api.test/memories/7/confirm'
	},
	{
		name: 'delete',
		call: (c) => deleteMemory(c, 7),
		ok: () => new Response(null, { status: 204 }),
		data: undefined,
		method: 'DELETE',
		url: 'http://api.test/memories/7'
	}
];

describe('memories api', () => {
	it.each(calls)('$name sends the right request and maps success', async (c) => {
		const { client, fetch } = setup(c.ok);
		expect(await c.call(client)).toEqual({ kind: 'ok', data: c.data });
		const request = await sent(fetch);
		expect(request.method).toBe(c.method);
		expect(request.url).toBe(c.url);
		if (c.body !== undefined) expect(await request.json()).toEqual(c.body);
	});

	it.each(calls)('$name maps 422, 404, 500 and a thrown fetch', async (c) => {
		const errors = [{ field: 'content', message: 'Required' }];
		const cases: [() => Response, Result<unknown>][] = [
			[() => json({ errors }, 422), { kind: 'invalid', errors }],
			[() => new Response(null, { status: 404 }), { kind: 'not-found' }],
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
			expect(await c.call(client)).toEqual(want);
		}
	});

	it('sends only the non-blank filters', async () => {
		const cases: [Parameters<typeof listMemories>[1], string][] = [
			[{ kind: '', status: '', tag: '  ', q: ' ' }, 'http://api.test/memories'],
			[{ status: 'proposed' }, 'http://api.test/memories?status=proposed'],
			[
				{ kind: 'answer', status: 'confirmed', tag: ' remote ', q: ' salary & equity ' },
				'http://api.test/memories?kind=answer&status=confirmed&tag=remote&q=salary%20%26%20equity'
			]
		];
		for (const [filters, url] of cases) {
			const { client, fetch } = setup(() => json([]));
			expect(await listMemories(client, filters)).toEqual({ kind: 'ok', data: [] });
			expect((await sent(fetch)).url).toBe(url);
		}
	});
});
