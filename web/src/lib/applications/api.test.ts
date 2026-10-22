import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type Application, type ApplicationEvent, type Stage } from '#lib/api';
import {
	addEntry,
	addStage,
	createApplication,
	deleteApplication,
	getApplication,
	listApplications,
	setApplicationResume,
	listEvents,
	listStages,
	removeStage,
	reorderStages,
	setApplicationStatus,
	setStageCompleted,
	updateApplication,
	type Result
} from './api';

const application: Application = {
	id: 3,
	company: 'Acme',
	title: 'Engineer',
	url: null,
	location: 'Remote',
	comp: null,
	source: null,
	notes: null,
	status: 'sourced',
	status_changed_at: '2026-10-22T00:00:00Z',
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:00Z',
	resume_version_id: null
};

const stages: Stage[] = [
	{
		id: 7,
		application_id: 3,
		name: 'Onsite',
		position: 0,
		completed_at: null,
		created_at: '2026-10-22T00:00:00Z'
	}
];

const note: ApplicationEvent = {
	id: 9,
	application_id: 3,
	kind: 'note',
	from_status: null,
	to_status: null,
	stage_name: null,
	body: 'Called back',
	created_at: '2026-10-22T00:00:00Z'
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
	const body = await request.text();
	return {
		method: request.method,
		url: request.url,
		contentType: request.headers.get('content-type'),
		body: body ? JSON.parse(body) : undefined
	};
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
		call: (c) => listApplications(c),
		ok: () => json([application]),
		data: [application],
		method: 'GET',
		url: 'http://api.test/applications'
	},
	{
		name: 'get',
		call: (c) => getApplication(c, 3),
		ok: () => json(application),
		data: application,
		method: 'GET',
		url: 'http://api.test/applications/3'
	},
	{
		name: 'create',
		call: (c) =>
			createApplication(c, {
				company: 'Acme',
				title: 'Engineer',
				status: 'applied'
			}),
		ok: () => json(application, 201),
		data: application,
		method: 'POST',
		url: 'http://api.test/applications',
		body: { company: 'Acme', title: 'Engineer', status: 'applied' }
	},
	{
		name: 'update',
		call: (c) =>
			updateApplication(c, 3, {
				company: 'Acme',
				title: 'Staff',
				location: null
			}),
		ok: () => json(application),
		data: application,
		method: 'PUT',
		url: 'http://api.test/applications/3',
		body: { company: 'Acme', title: 'Staff', location: null }
	},
	{
		name: 'set status',
		call: (c) => setApplicationStatus(c, 3, 'offer'),
		ok: () => json(application),
		data: application,
		method: 'PUT',
		url: 'http://api.test/applications/3/status',
		body: { status: 'offer' }
	},
	{
		name: 'set resume',
		call: (c) => setApplicationResume(c, 3, 7),
		ok: () => json(application),
		data: application,
		method: 'PUT',
		url: 'http://api.test/applications/3/resume',
		body: { resume_version_id: 7 }
	},
	{
		name: 'clear resume',
		call: (c) => setApplicationResume(c, 3, null),
		ok: () => json(application),
		data: application,
		method: 'PUT',
		url: 'http://api.test/applications/3/resume',
		body: { resume_version_id: null }
	},
	{
		name: 'set status with feedback',
		call: (c) => setApplicationStatus(c, 3, 'rejected', 'Too junior'),
		ok: () => json(application),
		data: application,
		method: 'PUT',
		url: 'http://api.test/applications/3/status',
		body: { status: 'rejected', feedback: 'Too junior' }
	},
	{
		name: 'delete',
		call: (c) => deleteApplication(c, 3),
		ok: () => new Response(null, { status: 204 }),
		data: undefined,
		method: 'DELETE',
		url: 'http://api.test/applications/3'
	},
	{
		name: 'list stages',
		call: (c) => listStages(c, 3),
		ok: () => json(stages),
		data: stages,
		method: 'GET',
		url: 'http://api.test/applications/3/stages'
	},
	{
		name: 'add stage',
		call: (c) => addStage(c, 3, 'Onsite'),
		ok: () => json(stages, 201),
		data: stages,
		method: 'POST',
		url: 'http://api.test/applications/3/stages',
		body: { name: 'Onsite' }
	},
	{
		name: 'complete stage',
		call: (c) => setStageCompleted(c, 3, 7, true),
		ok: () => json(stages),
		data: stages,
		method: 'PUT',
		url: 'http://api.test/applications/3/stages/7',
		body: { completed: true }
	},
	{
		name: 'reorder stages',
		call: (c) => reorderStages(c, 3, [8, 7]),
		ok: () => json(stages),
		data: stages,
		method: 'PUT',
		url: 'http://api.test/applications/3/stage-order',
		body: { stage_ids: [8, 7] }
	},
	{
		name: 'remove stage',
		call: (c) => removeStage(c, 3, 7),
		ok: () => json(stages),
		data: stages,
		method: 'DELETE',
		url: 'http://api.test/applications/3/stages/7'
	},
	{
		name: 'list events',
		call: (c) => listEvents(c, 3),
		ok: () => json([note]),
		data: [note],
		method: 'GET',
		url: 'http://api.test/applications/3/events'
	},
	{
		name: 'add entry',
		call: (c) => addEntry(c, 3, 'note', 'Called back'),
		ok: () => json(note, 201),
		data: note,
		method: 'POST',
		url: 'http://api.test/applications/3/events',
		body: { kind: 'note', body: 'Called back' }
	}
];

describe('applications api', () => {
	it.each(calls)('$name sends the right request and maps success', async (c) => {
		const { client, fetch } = setup(c.ok);
		expect(await c.call(client)).toEqual({ kind: 'ok', data: c.data });
		const request = await sent(fetch);
		expect(request.method).toBe(c.method);
		expect(request.url).toBe(c.url);
		expect(request.body).toEqual(c.body);
		if (c.body !== undefined) expect(request.contentType).toBe('application/json');
	});

	it.each(calls)('$name maps 422, 404, 409, 500 and a thrown fetch', async (c) => {
		const errors = [{ field: 'company', message: 'Required' }];
		const cases: [() => Response, Result<unknown>][] = [
			[() => json({ errors }, 422), { kind: 'invalid', errors }],
			[() => new Response(null, { status: 404 }), { kind: 'not-found' }],
			[() => new Response(null, { status: 409 }), { kind: 'conflict' }],
			[() => new Response('boom', { status: 500 }), { kind: 'failed' }],
			[() => json({ errors: [{ field: 'body', message: 'x' }] }, 400), { kind: 'failed' }],
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

	it('treats a success status other than the expected one as failed', async () => {
		const { client } = setup(() => json(application, 200));
		expect(await createApplication(client, { company: 'Acme', title: 'Engineer' })).toEqual({
			kind: 'failed'
		});
		for (const call of [
			() => addStage(client, 3, 'Onsite'),
			() => addEntry(client, 3, 'note', 'x')
		]) {
			expect(await call()).toEqual({ kind: 'failed' });
		}
	});
});
