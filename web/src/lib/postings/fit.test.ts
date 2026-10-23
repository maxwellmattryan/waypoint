import { describe, expect, it } from 'vitest';
import type { FitRun, FitScore, PostingFit } from '#lib/api';
import { fitView, type FitView } from './fit.js';

function score(id: number, runId: number, status: FitScore['status']): FitScore {
	return {
		id,
		run_id: runId,
		status,
		score: 70,
		band: 'partial',
		confidence: 0.8,
		summary: 'A fit',
		reasons: [],
		cost_usd: 0.01,
		created_at: '2026-10-22T10:00:00Z'
	};
}

const run = (id: number, status: FitRun['status']): FitRun => ({ id, status, error: null });
const applied = score(1, 5, 'applied');
const inReview = score(2, 8, 'in_review');

describe('fitView', () => {
	it.each<[string, Partial<PostingFit>, FitView['kind']]>([
		['nothing', {}, 'empty'],
		['a queued run', { current: applied, run: run(9, 'queued') }, 'scoring'],
		['a running run', { current: applied, run: run(9, 'running') }, 'scoring'],
		['a paused run', { current: applied, run: run(9, 'paused') }, 'paused'],
		['a failed run newer than every score', { current: applied, run: run(9, 'failed') }, 'failed'],
		[
			'a failed run older than the applied score',
			{ current: applied, run: run(4, 'failed') },
			'applied'
		],
		[
			'a failed run older than a score in review',
			{ in_review: inReview, run: run(6, 'failed') },
			'review'
		],
		['an applied score', { current: applied, run: run(5, 'succeeded') }, 'applied'],
		['only a score in review', { in_review: inReview, run: run(8, 'succeeded') }, 'review']
	])('picks the view for %s', (_, fit, kind) => {
		const full: PostingFit = { posting_id: 9, current: null, in_review: null, run: null, ...fit };
		expect(fitView(full).kind).toBe(kind);
	});

	it('marks an applied score that has a newer one in review', () => {
		const view = fitView({ posting_id: 9, current: applied, in_review: inReview, run: null });
		expect(view).toMatchObject({ kind: 'applied', newer: true });
	});
});
