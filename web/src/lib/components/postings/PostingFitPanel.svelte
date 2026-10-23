<script lang="ts" module>
	import type { PostingFitState } from '#lib/postings/postings.svelte.js';

	export interface PostingFitPanelProps {
		state: PostingFitState;
		/** A score request is in flight; the score buttons are disabled. */
		starting?: boolean;
		/** Queues a score: Score fit, Rescore and Try again after a failed run. */
		onscore: () => void;
		/** Loads the scores again after a load error. */
		onreload: () => void;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { fade, slide } from 'svelte/transition';
	import type { FitScore } from '#lib/api';
	import { formatDateTime } from '#lib/applications/format.js';
	import RunLink from '#lib/components/review/RunLink.svelte';
	import { Button, Card, Link, Skeleton } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import { confidenceLabel } from '#lib/postings/fit.js';
	import { formatCost } from '#lib/runs/format.js';
	import FitBadge from './FitBadge.svelte';
	import FitReasonList from './FitReasonList.svelte';

	/** The posting page's Fit section: every state of a posting's fit score. */
	let { state, starting = false, onscore, onreload }: PostingFitPanelProps = $props();

	const reviewHref = resolve('/review');
	const view = $derived(state.kind === 'loaded' ? state.view : null);
	/** Which block shows; a change fades between them. */
	const shown = $derived(view?.kind ?? state.kind);
	const scoreLabel = $derived.by(() => {
		if (!view || view.kind === 'empty') return 'Score fit';
		if (view.kind === 'scoring' || view.kind === 'paused') return 'Scoring…';
		if (view.kind === 'failed') return 'Try again';
		return 'Rescore';
	});
	const waiting = $derived(view?.kind === 'scoring' || view?.kind === 'paused');
</script>

{#snippet scoreBody(score: FitScore, provisional: boolean)}
	<div class={['flex flex-col gap-4', provisional && 'opacity-75']}>
		<div class="flex flex-wrap items-center gap-4">
			<p class="text-3xl font-semibold text-fg tabular-nums" aria-label="Score {score.score} of 100">
				{score.score}
			</p>
			<FitBadge score={score.score} band={score.band} inReview={provisional} />
			<span class="text-xs text-fg-muted tabular-nums">{confidenceLabel(score.confidence)}</span>
		</div>
		<p class="text-sm break-words text-fg">{score.summary}</p>
		<FitReasonList reasons={score.reasons} />
		<p class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-fg-muted">
			<span class="tabular-nums">
				Scored {formatDateTime(score.created_at)} · {formatCost(score.cost_usd)}
			</span>
			<RunLink runId={score.run_id} />
		</p>
	</div>
{/snippet}

<section class="flex flex-col gap-4" aria-labelledby="posting-fit-heading">
	<div class="flex flex-wrap items-center justify-between gap-4">
		<h2 id="posting-fit-heading" class="text-lg font-semibold text-fg">Fit</h2>
		{#if state.kind === 'loaded'}
			<Button
				variant={view?.kind === 'empty' ? 'primary' : 'secondary'}
				size="sm"
				disabled={starting || waiting}
				onclick={onscore}
			>
				{scoreLabel}
			</Button>
		{/if}
	</div>

	{#key shown}
		<div in:fade={{ duration: motionDuration('base') }}>
			{#if state.kind === 'loading'}
				<div aria-busy="true" aria-label="Loading fit score">
					<Skeleton class="h-24" />
				</div>
			{:else if state.kind === 'error'}
				<Card class="flex flex-col items-start gap-2">
					<p class="text-sm font-semibold text-fg">Couldn't load the fit score</p>
					<p class="text-sm text-fg-muted">Check that the API is running, then try again.</p>
					<Button variant="secondary" size="sm" onclick={onreload}>Try again</Button>
				</Card>
			{:else if view?.kind === 'empty'}
				<Card class="flex flex-col gap-1">
					<p class="text-sm font-semibold text-fg">Not scored yet</p>
					<p class="text-sm text-fg-muted">
						Score this posting against what Waypoint knows about you.
					</p>
				</Card>
			{:else if view?.kind === 'scoring' || view?.kind === 'paused'}
				<Card class="flex flex-col gap-4" aria-busy="true">
					<div class="flex flex-wrap items-center gap-x-4 gap-y-1">
						<p class="text-sm font-semibold text-fg" role="status">
							{view.kind === 'paused' ? 'Paused: agents are paused' : 'Scoring…'}
						</p>
						<RunLink runId={view.run.id} />
					</div>
					<div class="flex flex-col gap-2" aria-hidden="true">
						<Skeleton class="h-8 w-32" />
						<Skeleton class="h-4" />
						<Skeleton class="h-4 w-2/3" />
					</div>
				</Card>
			{:else if view?.kind === 'failed'}
				<Card class="flex flex-col gap-2">
					<div class="flex flex-wrap items-center gap-x-4 gap-y-1">
						<p class="text-sm font-semibold text-danger" role="alert">Scoring failed</p>
						<RunLink runId={view.run.id} />
					</div>
					{#if view.run.error}
						<p class="text-sm break-words whitespace-pre-wrap text-fg-muted">{view.run.error}</p>
					{/if}
				</Card>
			{:else if view?.kind === 'applied'}
				<Card class="flex flex-col gap-4">
					{#if view.newer}
						<div
							class="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-md bg-warning-soft px-4 py-2 text-sm text-warning-soft-fg"
							transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}
						>
							<span>A newer score is waiting in review</span>
							<Link href={reviewHref}>Open the review queue</Link>
						</div>
					{/if}
					{@render scoreBody(view.score, false)}
				</Card>
			{:else if view?.kind === 'review'}
				<Card class="flex flex-col gap-4">
					<p class="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-fg-muted">
						<span>This score is waiting in review and isn't applied yet.</span>
						<Link href={reviewHref}>Open the review queue</Link>
					</p>
					{@render scoreBody(view.score, true)}
				</Card>
			{/if}
		</div>
	{/key}
</section>
