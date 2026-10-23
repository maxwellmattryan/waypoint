<script lang="ts">
	import { resolve } from '$app/paths';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import type { ShortlistEntry } from '#lib/api';
	import FitBadge from '#lib/components/postings/FitBadge.svelte';
	import { Link } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';

	/** The shortlisted postings, by rank. */
	let { entries }: { entries: ShortlistEntry[] } = $props();
</script>

<ol class="flex flex-col divide-y divide-border" aria-label="Shortlist">
	{#each entries as entry (entry.posting_id)}
		<li
			class="flex flex-col gap-2 py-2 sm:flex-row sm:items-center sm:justify-between sm:gap-4"
			animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
			in:fade={{ duration: motionDuration('base') }}
		>
			<div class="flex min-w-0 items-baseline gap-2">
				<span class="text-sm font-semibold text-fg-muted tabular-nums">{entry.rank}.</span>
				<span class="flex min-w-0 flex-col gap-1">
					<Link href={resolve('/postings/[id]', { id: String(entry.posting_id) })} class="text-sm break-words">
						{entry.title}
					</Link>
					<span class="text-xs break-words text-fg-muted">{entry.company}</span>
				</span>
			</div>
			<FitBadge score={entry.score} band={entry.band} />
		</li>
	{/each}
</ol>
