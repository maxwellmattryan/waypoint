import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type ResumeVersion, type ResumeVersionDetail } from '#lib/api';
import { deleteResume, getResume, listResumes, pdfUrl, uploadResume, type Result } from './api';

const version: ResumeVersion = {
	id: 7,
	name: 'Backend',
	filename: 'backend.pdf',
	size_bytes: 1234,
	sha256: 'ab'.repeat(32),
	has_text: true,
	application_count: 0,
	created_at: '2026-10-22T00:00:00Z'
};
const detail: ResumeVersionDetail = { ...version, text: 'Waypoint Fixture Resume' };

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

const pdf = () => new File(['%PDF-1.4 fake'], 'My CV & co.pdf', { type: 'application/pdf' });

type Call = (client: ReturnType<typeof setup>['client']) => Promise<Result<unknown>>;

const calls: { name: string; call: Call; ok: () => Response; data: unknown; method: string; url: string }[] = [
	{
		name: 'list',
		call: (c) => listResumes(c),
		ok: () => json([version]),
		data: [version],
		method: 'GET',
		url: 'http://api.test/resumes'
	},
	{
		name: 'get',
		call: (c) => getResume(c, 7),
		ok: () => json(detail),
		data: detail,
		method: 'GET',
		url: 'http://api.test/resumes/7'
	},
	{
		name: 'upload',
		call: (c) => uploadResume(c, pdf(), 'Backend & Rust'),
		ok: () => json(version, 201),
		data: version,
		method: 'POST',
		url: 'http://api.test/resumes?name=Backend%20%26%20Rust&filename=My%20CV%20%26%20co.pdf'
	},
	{
		name: 'delete',
		call: (c) => deleteResume(c, 7),
		ok: () => new Response(null, { status: 204 }),
		data: undefined,
		method: 'DELETE',
		url: 'http://api.test/resumes/7'
	}
];

describe('resumes api', () => {
	it.each(calls)('$name sends the right request and maps success', async (c) => {
		const { client, fetch } = setup(c.ok);
		expect(await c.call(client)).toEqual({ kind: 'ok', data: c.data });
		const request = await sent(fetch);
		expect(request.method).toBe(c.method);
		expect(request.url).toBe(c.url);
	});

	it.each(calls)('$name maps 422, 404, 409, 500 and a thrown fetch', async (c) => {
		const errors = [{ field: 'name', message: 'Required' }];
		const cases: [() => Response, Result<unknown>][] = [
			[() => json({ errors }, 422), { kind: 'invalid', errors }],
			[() => new Response(null, { status: 404 }), { kind: 'not-found' }],
			[() => new Response(null, { status: 409 }), { kind: 'conflict' }],
			[() => new Response('boom', { status: 500 }), { kind: 'failed' }],
			[() => new Response(null, { status: 415 }), { kind: 'failed' }],
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

	it('uploads the raw file as application/pdf', async () => {
		const { client, fetch } = setup(() => json(version, 201));
		const file = pdf();
		await uploadResume(client, file, 'Backend');
		const request = await sent(fetch);
		expect(request.headers.get('content-type')).toBe('application/pdf');
		expect(new Uint8Array(await request.arrayBuffer())).toEqual(
			new Uint8Array(await file.arrayBuffer())
		);
	});

	it('maps a 413 upload to a file error', async () => {
		const { client } = setup(() => new Response(null, { status: 413 }));
		expect(await uploadResume(client, pdf(), 'Backend')).toEqual({
			kind: 'invalid',
			errors: [{ field: 'file', message: 'At most 10 MB' }]
		});
	});

	it('builds the PDF link through the API proxy', () => {
		expect(pdfUrl(7, 'http://127.0.0.1:5173/api')).toBe('http://127.0.0.1:5173/api/resumes/7/pdf');
	});
});
