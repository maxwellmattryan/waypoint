<script lang="ts">
	import { resolve } from '$app/paths';
	import type { JobPosting } from '#lib/api';
	import { formatDate } from '#lib/applications/format.js';
	import { Card } from '#lib/components/ui';
	import FitBadge from './FitBadge.svelte';
	import PostingBadges from './PostingBadges.svelte';

	let { posting }: { posting: JobPosting } = $props();

	const href = $derived(resolve('/postings/[id]', { id: String(posting.id) }));
	const location = $derived(posting.location_raw || (posting.remote ? 'Remote' : ''));
	const place = $derived(location ? `${posting.company} · ${location}` : posting.company);
	const seen = $derived(
		posting.posted_at
			? `Posted ${formatDate(posting.posted_at)}`
			: `First seen ${formatDate(posting.first_seen_at)}`
	);
</script>

<Card {href}>
	<div class="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<p class="text-sm font-semibold break-words text-fg">{posting.title}</p>
			<p class="text-sm break-words text-fg-muted">{place}</p>
			<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted">
				{#if posting.department}
					<span>{posting.department}</span>
				{/if}
				<span>{seen}</span>
			</p>
		</div>
		<div class="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end">
			<FitBadge score={posting.fit_score} band={posting.fit_band} inReview={posting.fit_in_review} />
			<PostingBadges {posting} />
		</div>
	</div>
</Card>
