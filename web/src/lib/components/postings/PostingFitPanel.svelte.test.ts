import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { FitRun, FitScore, PostingFit } from '#lib/api';
import { memoryHref } from '#lib/memories/link.js';
import { fitView } from '#lib/postings/fit.js';
import type { PostingFitState } from '#lib/postings/postings.svelte.js';
import PostingFitPanel from './PostingFitPanel.svelte';

function score(overrides: Partial<FitScore> = {}): FitScore {
	return {
		id: 1,
		run_id: 5,
		status: 'applied',
		score: 82,
		band: 'strong',
		confidence: 0.9,
		summary: 'A strong Rust match',
		reasons: [
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
				text: 'Wants an office',
				citations: [
					{
						source: 'posting',
						memory_id: null,
						topic: null,
						text: '<b>On-site</b> in Berlin'
					},
					{
						source: 'memory',
						memory_id: null,
						topic: 'location',
						text: 'Remote only'
					}
				]
			}
		],
		cost_usd: 0.0123,
		created_at: '2026-10-22T10:00:00Z',
		...overrides
	};
}

const run = (status: FitRun['status'], id = 5, error: string | null = null): FitRun => ({
	id,
	status,
	error
});

function loaded(fit: Partial<PostingFit>): PostingFitState {
	const full: PostingFit = {
		posting_id: 9,
		current: null,
		in_review: null,
		run: null,
		...fit
	};
	return { kind: 'loaded', fit: full, view: fitView(full) };
}

function setup(state: PostingFitState, starting = false) {
	const onscore = vi.fn();
	const onreload = vi.fn();
	const result = render(PostingFitPanel, {
		state,
		starting,
		onscore,
		onreload
	});
	return { ...result, onscore, onreload };
}

const button = (name: string) => screen.getByRole('button', { name });

describe('PostingFitPanel', () => {
	it('shows the never-scored state with Score fit', async () => {
		const { onscore } = setup(loaded({}));
		expect(screen.getByText('Not scored yet')).toBeInTheDocument();
		await userEvent.click(button('Score fit'));
		expect(onscore).toHaveBeenCalledTimes(1);
	});

	it.each([
		['queued', 'Scoring…'],
		['running', 'Scoring…'],
		['paused', 'Paused: agents are paused']
	] as const)('shows %s as scoring, with the button disabled', (status, text) => {
		setup(loaded({ current: score(), run: run(status, 6) }));
		expect(screen.getByRole('status')).toHaveTextContent(text);
		expect(button('Scoring…')).toBeDisabled();
		expect(screen.queryByText('A strong Rust match')).toBeNull();
	});

	it('shows a failed run with its error, its run and Try again', async () => {
		const { onscore } = setup(
			loaded({
				current: score({ run_id: 5 }),
				run: run('failed', 6, 'step "score": boom')
			})
		);
		expect(screen.getByRole('alert')).toHaveTextContent('Scoring failed');
		expect(screen.getByText('step "score": boom')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Run #6' })).toHaveAttribute('href', '/runs/6');
		await userEvent.click(button('Try again'));
		expect(onscore).toHaveBeenCalledTimes(1);
	});

	it('shows the applied score when a failed run is older than it', () => {
		setup(loaded({ current: score({ run_id: 8 }), run: run('failed', 6, 'boom') }));
		expect(screen.queryByText('Scoring failed')).toBeNull();
		expect(screen.getByText('A strong Rust match')).toBeInTheDocument();
	});

	it('shows the applied score with its reasons, citations and cost', () => {
		const { container } = setup(loaded({ current: score(), run: run('succeeded') }));
		expect(screen.getByText('82 · Strong fit')).toBeInTheDocument();
		expect(screen.getByText('90% confident')).toBeInTheDocument();
		expect(screen.getByText('A strong Rust match')).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'For' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Against' })).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'View memory' })).toHaveAttribute(
			'href',
			memoryHref(7)
		);
		expect(screen.getByText('Deleted memory')).toBeInTheDocument();
		// A posting excerpt is literal text.
		expect(container.querySelector('blockquote b')).toBeNull();
		expect(screen.getByText(/<b>On-site<\/b> in Berlin/)).toBeInTheDocument();
		expect(screen.getByText(/Scored .* · \$0\.01/)).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Run #5' })).toBeInTheDocument();
		expect(button('Rescore')).toBeEnabled();
		expect(screen.queryByText('Needs review')).toBeNull();
		expect(screen.queryByText('A newer score is waiting in review')).toBeNull();
	});

	it('shows a provisional score in review when nothing is applied', () => {
		setup(
			loaded({
				in_review: score({
					id: 2,
					status: 'in_review',
					score: 55,
					band: 'partial'
				}),
				run: run('succeeded')
			})
		);
		expect(screen.getByText('55 · Partial fit')).toBeInTheDocument();
		expect(screen.getByText('Needs review')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Open the review queue' })).toHaveAttribute(
			'href',
			'/review'
		);
	});

	it('shows the applied score with a notice when a newer one is in review', () => {
		setup(
			loaded({
				current: score(),
				in_review: score({
					id: 2,
					run_id: 6,
					status: 'in_review',
					score: 40,
					band: 'partial'
				}),
				run: run('succeeded', 6)
			})
		);
		expect(screen.getByText('82 · Strong fit')).toBeInTheDocument();
		expect(screen.queryByText('40 · Partial fit')).toBeNull();
		expect(screen.getByText('A newer score is waiting in review')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Open the review queue' })).toHaveAttribute(
			'href',
			'/review'
		);
	});

	it('disables the button while a score request is in flight', () => {
		setup(loaded({ current: score() }), true);
		expect(button('Rescore')).toBeDisabled();
	});

	it('shows a load error with Try again', async () => {
		const { onreload, onscore } = setup({ kind: 'error' });
		expect(screen.getByText("Couldn't load the fit score")).toBeInTheDocument();
		await userEvent.click(button('Try again'));
		expect(onreload).toHaveBeenCalledTimes(1);
		expect(onscore).not.toHaveBeenCalled();
	});

	it('shows a skeleton while loading', () => {
		setup({ kind: 'loading' });
		expect(screen.getByLabelText('Loading fit score')).toBeInTheDocument();
		expect(screen.queryByRole('button')).toBeNull();
	});
});
