import type { ApiClient, FitBand, FitRun, FitScore, PostingFit, ScoreFitQueued } from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';
import type { BadgeTone } from '#lib/components/ui';

/** How each band reads on a badge. */
export const FIT_BAND_LABELS: Record<FitBand, string> = {
	strong: 'Strong fit',
	partial: 'Partial fit',
	weak: 'Weak fit'
};

/** Each band's badge tone. */
export const FIT_BAND_TONES: Record<FitBand, BadgeTone> = {
	strong: 'success',
	partial: 'info',
	weak: 'neutral'
};

/** The `score_fit` workflow's name, as `run.updated` events carry it. */
export const SCORE_FIT_WORKFLOW = 'score_fit';

/** A posting's scores and latest scoring run; a 404 is `not-found`. */
export function getPostingFit(client: ApiClient, id: number): Promise<Result<PostingFit>> {
	return run(200, () => client.GET('/postings/{id}/fit', { params: { path: { id } } }));
}

/** Queues a `score_fit` run for a posting, or gets back the one already waiting or running. */
export function scorePosting(client: ApiClient, id: number): Promise<Result<ScoreFitQueued>> {
	return run(202, () => client.POST('/postings/{id}/fit', { params: { path: { id } }, body: {} }));
}

/** What the posting page's Fit section shows. */
export type FitView =
	| { kind: 'empty' }
	| { kind: 'scoring'; run: FitRun }
	| { kind: 'paused'; run: FitRun }
	| { kind: 'failed'; run: FitRun }
	/** The applied score; `newer` when a newer score waits in review. */
	| { kind: 'applied'; score: FitScore; newer: boolean }
	/** Only a provisional score, waiting in review. */
	| { kind: 'review'; score: FitScore };

/**
 * The Fit section's state: a waiting or running run first, then a failed run no score is newer
 * than, then the applied score (with a notice when a newer one waits in review), then a score
 * waiting in review, else nothing.
 */
export function fitView(fit: PostingFit): FitView {
	const { current, in_review: inReview, run: latest } = fit;
	if (latest) {
		if (latest.status === 'paused') return { kind: 'paused', run: latest };
		if (latest.status === 'queued' || latest.status === 'running') {
			return { kind: 'scoring', run: latest };
		}
		if (latest.status === 'failed') {
			const newest = Math.max(current?.run_id ?? 0, inReview?.run_id ?? 0);
			if (newest < latest.id) return { kind: 'failed', run: latest };
		}
	}
	if (current) return { kind: 'applied', score: current, newer: inReview !== null };
	if (inReview) return { kind: 'review', score: inReview };
	return { kind: 'empty' };
}

/** A score as a percentage of confidence, e.g. "82% confident". */
export function confidenceLabel(confidence: number): string {
	return `${Math.round(confidence * 100)}% confident`;
}
