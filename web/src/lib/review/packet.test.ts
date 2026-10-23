import { describe, expect, it } from 'vitest';
import type { ReviewItem } from '#lib/api';
import { makeAnswers } from '#lib/packets/__fixtures__/packets.js';
import { describePacketReview, PACKET_KIND } from './packet';

type Payload = Record<string, unknown>;

function payload(overrides: Payload = {}): Payload {
	const answers = makeAnswers();
	answers[2] = { ...answers[2], answer: null, citations: [] };
	return {
		posting_id: 9,
		company: 'Acme',
		title: 'Backend Engineer',
		resume: { id: 1, name: 'Backend', reasons: [] },
		answers,
		confidence: 0.9,
		missing: ['salary_expectation'],
		dropped: { resume_reasons: 0, citations: 0, answers: 0 },
		...overrides
	};
}

function item(body: unknown, kind = PACKET_KIND): ReviewItem {
	return {
		id: 4,
		run_id: 50,
		run_step_id: 7,
		kind,
		confidence: 0.9,
		payload: body as ReviewItem['payload'],
		status: 'pending',
		created_at: '2026-10-22T00:00:00Z',
		resolved_at: null
	};
}

describe('describePacketReview', () => {
	it('reads the posting, the resume, the missing questions and the drafted answers', () => {
		const review = describePacketReview(item(payload()));
		expect(review).toMatchObject({
			badge: { label: 'Missing answers', tone: 'warning' },
			postingId: 9,
			company: 'Acme',
			title: 'Backend Engineer',
			resumeName: 'Backend',
			missing: ['What are your salary expectations?']
		});
		expect(review?.answers.map((answer) => answer.key)).toEqual([
			'work_authorization',
			'notice_period',
			'why_leaving',
			'why_company'
		]);
		expect(review?.answers[0]).toEqual({
			key: 'work_authorization',
			question:
				"Are you authorized to work in the role's location, and will you need visa sponsorship?",
			answer: 'Yes. I am an EU citizen.',
			citations: [
				{
					source: 'memory',
					memory_id: 1,
					topic: 'work_authorization',
					text: 'EU citizen'
				}
			]
		});
	});

	it.each([
		[
			'dropped citations',
			{ dropped: { resume_reasons: 0, citations: 1, answers: 0 } },
			'Unchecked citations'
		],
		[
			'a dropped answer',
			{ dropped: { resume_reasons: 0, citations: 0, answers: 1 } },
			'Unchecked citations'
		],
		['only low confidence', { answers: makeAnswers(), missing: [] }, 'Unsure packet']
	])('labels %s', (_, overrides, label) => {
		expect(describePacketReview(item(payload(overrides)))?.badge.label).toBe(label);
	});

	it.each([
		['another kind', item(payload(), 'score_fit.score')],
		['a non-object payload', item('nope')],
		['no posting id', item(payload({ posting_id: '9' }))],
		['no company', item(payload({ company: null }))],
		['no resume name', item(payload({ resume: { id: 1 } }))],
		['no answers', item(payload({ answers: 'none' }))],
		['an unknown key', item(payload({ answers: [{ ...makeAnswers()[0], key: 'cover_letter' }] }))],
		['a non-text answer', item(payload({ answers: [{ ...makeAnswers()[0], answer: 5 }] }))],
		[
			'a bad citation',
			item(
				payload({
					answers: [{ ...makeAnswers()[0], citations: [{ source: 'web', text: 'x' }] }]
				})
			)
		]
	])('is null for %s', (_, review) => {
		expect(describePacketReview(review)).toBeNull();
	});
});
