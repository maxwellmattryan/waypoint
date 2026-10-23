import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type MemorySignal, type ReviewItem } from '#lib/api';
import { PAGE_SIZE } from '#lib/runs/api.js';
import { dismissReviewItem, listSignals } from './api';

const item: ReviewItem = {
	id: 4,
	run_id: 9,
	run_step_id: 2,
	kind: 'tidy.note',
	confidence: 0.3,
	payload: { note: 'n' },
	status: 'dismissed',
	created_at: '2026-10-22T00:00:00Z',
	resolved_at: '2026-10-22T00:01:00Z'
};

const signal: MemorySignal = {
	id: 7,
	kind: 'dismissal',
	run_id: 9,
	pending_action_id: null,
	review_item_id: 4,
	subject: 'tidy.note',
	drafted: { note: 'n' },
	approved: null,
	changes: null,
	reason: 'Wrong role',
	created_at: '2026-10-22T00:01:00Z'
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

describe('review api', () => {
	it.each([
		['without a reason', undefined, {}],
		['with a reason', 'Not relevant', { reason: 'Not relevant' }]
	])('dismissReviewItem sends the reason only when given (%s)', async (_, reason, body) => {
		const { client, fetch } = setup(() => json(item));
		expect(await dismissReviewItem(client, 4, reason)).toEqual({
			kind: 'ok',
			data: item
		});
		expect(await sent(fetch)).toEqual({
			method: 'POST',
			url: 'http://api.test/review-items/4/dismiss',
			contentType: 'application/json',
			body
		});
	});

	it('dismissReviewItem is invalid on a 422', async () => {
		const errors = [{ field: 'reason', message: 'At most 1000 characters' }];
		const { client } = setup(() => json({ errors }, 422));
		expect(await dismissReviewItem(client, 4, 'x')).toEqual({
			kind: 'invalid',
			errors
		});
	});

	it.each([
		[{}, `http://api.test/memory-signals?limit=${PAGE_SIZE}`],
		[
			{ limit: 2, before: 7, kind: 'approval' as const },
			'http://api.test/memory-signals?limit=2&before=7&kind=approval'
		]
	])('listSignals sends %o', async (options, url) => {
		const { client, fetch } = setup(() => json([signal]));
		expect(await listSignals(client, options)).toEqual({
			kind: 'ok',
			data: [signal]
		});
		expect(await sent(fetch)).toMatchObject({ method: 'GET', url });
	});
});
