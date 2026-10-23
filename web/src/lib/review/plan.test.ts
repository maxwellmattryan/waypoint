import { describe, expect, it } from 'vitest';
import type { ReviewItem } from '#lib/api';
import { describePlanReview, PLAN_KIND } from './plan';

function payload(overrides: Record<string, unknown> = {}) {
	return {
		goal_id: 3,
		plan_id: 12,
		version: 2,
		goal: 'Frontend roles at climate companies',
		summary: 'Two climate companies in NYC',
		companies: [
			{
				name: 'Example Labs',
				source: 'greenhouse',
				board: 'example-labs',
				reason: 'Grid software',
				verified: true
			},
			{ name: 'Watershed', source: 'ashby', board: 'watershed', reason: null, verified: false }
		],
		dropped: { companies: [], keywords: [], locations: [] },
		...overrides
	};
}

function item(value: unknown, kind = PLAN_KIND): ReviewItem {
	return {
		id: 1,
		run_id: 2,
		run_step_id: 3,
		kind,
		confidence: 0.6,
		payload: value,
		status: 'pending',
		created_at: '2026-10-22T00:00:00Z',
		resolved_at: null
	};
}

describe('describePlanReview', () => {
	it('reads an unsure plan', () => {
		expect(describePlanReview(item(payload()))).toEqual({
			badge: { label: 'Unsure plan', tone: 'warning' },
			goalId: 3,
			version: 2,
			goal: 'Frontend roles at climate companies',
			summary: 'Two climate companies in NYC',
			companies: [
				{
					name: 'Example Labs',
					source: 'greenhouse',
					board: 'example-labs',
					reason: 'Grid software',
					verified: true
				},
				{ name: 'Watershed', source: 'ashby', board: 'watershed', reason: null, verified: false }
			],
			dropped: { companies: [], keywords: [], locations: [] }
		});
	});

	it('reads the dropped items', () => {
		const dropped = {
			companies: [{ name: 'Bad', source: 'greenhouse', board: 'bad slug!', why: 'board: Must be…' }],
			keywords: ['x'.repeat(51)],
			locations: []
		};
		const review = describePlanReview(item(payload({ dropped })));
		expect(review?.badge).toEqual({ label: 'Plan with dropped items', tone: 'warning' });
		expect(review?.dropped).toEqual(dropped);
	});

	it.each([
		['another kind', item(payload(), 'score_fit.score')],
		['a non-object payload', item([1, 2])],
		['no goal id', item(payload({ goal_id: 'three' }))],
		['a company with an unknown source', item(payload({ companies: [{ name: 'A', source: 'workday', board: 'a' }] }))],
		['a malformed dropped company', item(payload({ dropped: { companies: [{ name: 'A' }] } }))],
		['dropped keywords that are not text', item(payload({ dropped: { keywords: [1] } }))]
	])('is null for %s', (_, value) => {
		expect(describePlanReview(value)).toBeNull();
	});
});
