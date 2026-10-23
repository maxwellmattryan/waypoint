import { describe, expect, it } from 'vitest';
import type { ReviewItem } from '#lib/api';
import { describeFitReview, FIT_KIND } from './fit';

const reasons = [
	{
		stance: 'for',
		text: 'Has shipped Rust services',
		citations: [
			{
				source: 'memory',
				memory_id: 7,
				topic: 'skill',
				text: 'Shipped Rust services'
			}
		]
	},
	{
		stance: 'against',
		text: 'Level unclear',
		citations: [{ source: 'posting', memory_id: null, topic: null, text: 'Engineer' }]
	}
];

function payload(overrides: Record<string, unknown> = {}) {
	return {
		posting_id: 9,
		company: 'Acme',
		title: 'Backend Engineer',
		score: 55,
		band: 'partial',
		confidence: 0.5,
		summary: 'Unclear seniority',
		reasons,
		dropped: { reasons: 0, citations: 0 },
		...overrides
	};
}

function item(value: unknown, kind = FIT_KIND): ReviewItem {
	return {
		id: 3,
		run_id: 4,
		run_step_id: 5,
		kind,
		confidence: 0.5,
		payload: value,
		status: 'pending',
		created_at: '2026-10-22T00:00:00Z',
		resolved_at: null
	};
}

describe('describeFitReview', () => {
	it('reads a fit review item', () => {
		expect(describeFitReview(item(payload()))).toEqual({
			badge: { label: 'Unsure fit score', tone: 'warning' },
			postingId: 9,
			company: 'Acme',
			title: 'Backend Engineer',
			score: 55,
			band: 'partial',
			summary: 'Unclear seniority',
			reasons
		});
	});

	it('names dropped citations in the badge', () => {
		const described = describeFitReview(item(payload({ dropped: { reasons: 1, citations: 0 } })));
		expect(described?.badge).toEqual({
			label: 'Unchecked citations',
			tone: 'warning'
		});
	});

	it.each([
		['another kind', item(payload(), 'email.match')],
		['a non-object payload', item([1, 2])],
		['a missing company', item(payload({ company: undefined }))],
		['an unknown band', item(payload({ band: 'great' }))],
		['a string score', item(payload({ score: '55' }))],
		['reasons that are not a list', item(payload({ reasons: {} }))],
		['an unknown stance', item(payload({ reasons: [{ ...reasons[0], stance: 'maybe' }] }))],
		[
			'an unknown citation source',
			item(
				payload({
					reasons: [
						{
							...reasons[0],
							citations: [{ source: 'web', memory_id: null, topic: null, text: 'x' }]
						}
					]
				})
			)
		],
		[
			'an unknown memory topic',
			item(
				payload({
					reasons: [
						{
							...reasons[0],
							citations: [{ source: 'memory', memory_id: 1, topic: 'mood', text: 'x' }]
						}
					]
				})
			)
		]
	])('returns null for %s', (_, value) => {
		expect(describeFitReview(value)).toBeNull();
	});
});
