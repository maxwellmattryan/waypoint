<script lang="ts" module>
	import type { FitReason } from '#lib/api';

	export interface FitReasonListProps {
		reasons: FitReason[];
	}
</script>

<script lang="ts">
	import CitationList from './CitationList.svelte';

	/** A fit score's reasons in a "For" and an "Against" group, each with its citations. */
	let { reasons }: FitReasonListProps = $props();

	const groups = $derived(
		[
			{ label: 'For', items: reasons.filter((reason) => reason.stance === 'for') },
			{ label: 'Against', items: reasons.filter((reason) => reason.stance === 'against') }
		].filter((group) => group.items.length > 0)
	);
</script>

<div class="flex flex-col gap-4">
	{#each groups as group (group.label)}
		<section class="flex flex-col gap-2" aria-label="Reasons {group.label.toLowerCase()}">
			<h3 class="text-sm font-semibold text-fg">{group.label}</h3>
			<ul class="flex flex-col gap-4">
				{#each group.items as reason, i (i)}
					<li class="flex flex-col gap-2">
						<p class="text-sm break-words text-fg">{reason.text}</p>
						<CitationList citations={reason.citations} />
					</li>
				{/each}
			</ul>
		</section>
	{/each}
</div>
