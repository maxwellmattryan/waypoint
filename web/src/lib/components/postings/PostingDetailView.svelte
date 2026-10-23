<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { JobPostingDetail } from '#lib/api';
	import { formatDateTime, isWebUrl } from '#lib/applications/format.js';
	import { Card, KeyValue, Link, type KeyValueItem } from '#lib/components/ui';
	import { SOURCE_LABELS } from '#lib/postings/api.js';
	import PostingBadges from './PostingBadges.svelte';

	/** `fit` renders between the header and the details: the posting's Fit section. */
	let { posting, fit }: { posting: JobPostingDetail; fit?: Snippet } = $props();

	const NONE = '—';
	const text = (value: string | null) => value || NONE;
	const when = (value: string | null) => (value ? formatDateTime(value) : NONE);

	const viewUrl = $derived(isWebUrl(posting.url) ? posting.url : null);
	const applyUrl = $derived(
		posting.apply_url && posting.apply_url !== posting.url && isWebUrl(posting.apply_url)
			? posting.apply_url
			: null
	);

	const details = $derived.by(() => {
		const parsed = [posting.city, posting.region, posting.country].filter(Boolean).join(', ');
		const items: KeyValueItem[] = [
			{ key: 'Company', value: posting.company },
			{ key: 'Board', value: `${SOURCE_LABELS[posting.source]} · ${posting.board}` },
			{ key: 'Department', value: text(posting.department) },
			{ key: 'Employment type', value: text(posting.employment_type) },
			{ key: 'Location', value: text(posting.location_raw) },
			{ key: 'Parsed location', value: text(parsed) },
			{ key: 'Remote', value: posting.remote ? 'Yes' : 'No' },
			{ key: 'Posted', value: when(posting.posted_at) },
			{ key: 'First seen', value: when(posting.first_seen_at) },
			{ key: 'Last seen', value: when(posting.last_seen_at) }
		];
		if (posting.removed_at) items.push({ key: 'Removed', value: when(posting.removed_at) });
		return items;
	});
</script>

<div class="flex flex-col gap-8">
	<header class="flex flex-col gap-2">
		<h1 class="text-2xl font-semibold break-words text-fg">{posting.title}</h1>
		<p class="text-base break-words text-fg-muted">{posting.company}</p>
		<PostingBadges {posting} />
		{#if viewUrl || applyUrl}
			<div class="flex flex-wrap gap-4 text-sm">
				{#if viewUrl}
					<Link href={viewUrl} external>View posting</Link>
				{/if}
				{#if applyUrl}
					<Link href={applyUrl} external>Apply</Link>
				{/if}
			</div>
		{/if}
	</header>

	{@render fit?.()}

	<Card>
		<KeyValue items={details} />
	</Card>

	<section class="flex flex-col gap-4" aria-labelledby="posting-description-heading">
		<h2 id="posting-description-heading" class="text-lg font-semibold text-fg">Description</h2>
		{#if posting.description}
			<Card>
				<div class="text-sm leading-6 break-words whitespace-pre-wrap text-fg">{posting.description}</div>
			</Card>
		{:else}
			<p class="text-sm text-fg-muted">The board gave no description.</p>
		{/if}
	</section>
</div>
