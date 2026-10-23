<script lang="ts" module>
	import type { FitReview } from '#lib/review/fit.js';

	export interface FitReviewSummaryProps {
		summary: FitReview;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import FitBadge from '#lib/components/postings/FitBadge.svelte';
	import FitReasonList from '#lib/components/postings/FitReasonList.svelte';
	import { KeyValue, Link, type KeyValueItem } from '#lib/components/ui';

	/** A fit-score review item, readable: the posting, the score, the summary and the reasons. */
	let { summary }: FitReviewSummaryProps = $props();

	const postingHref = $derived(resolve('/postings/[id]', { id: String(summary.postingId) }));

	const items = $derived<KeyValueItem[]>([
		{ key: 'Posting', value: posting },
		{ key: 'Score', value: score },
		{ key: 'Summary', value: summary.summary },
		{ key: 'Reasons', value: reasons }
	]);
</script>

{#snippet posting()}
	<Link href={postingHref}>{summary.company} — {summary.title}</Link>
{/snippet}

{#snippet score()}
	<FitBadge score={summary.score} band={summary.band} />
{/snippet}

{#snippet reasons()}
	<FitReasonList reasons={summary.reasons} />
{/snippet}

<KeyValue {items} />
