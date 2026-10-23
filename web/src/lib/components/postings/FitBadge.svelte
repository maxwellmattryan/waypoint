<script lang="ts" module>
	import type { FitBand } from '#lib/api';

	export interface FitBadgeProps {
		/** The score, 0 to 100; null when there is none to show. */
		score: number | null;
		band: FitBand | null;
		/** Adds a `Needs review` badge: a newer score waits in the review queue. */
		inReview?: boolean;
	}
</script>

<script lang="ts">
	import { Badge } from '#lib/components/ui';
	import { FIT_BAND_LABELS, FIT_BAND_TONES } from '#lib/postings/fit.js';

	/** A fit score as a badge, `82 · Strong fit`, with an optional `Needs review` badge. */
	let { score, band, inReview = false }: FitBadgeProps = $props();
</script>

{#if (score !== null && band !== null) || inReview}
	<span class="inline-flex flex-wrap items-center gap-2">
		{#if score !== null && band !== null}
			<Badge tone={FIT_BAND_TONES[band]} class="tabular-nums">{score} · {FIT_BAND_LABELS[band]}</Badge>
		{/if}
		{#if inReview}
			<Badge tone="warning">Needs review</Badge>
		{/if}
	</span>
{/if}
