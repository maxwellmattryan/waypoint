<script lang="ts" module>
	import type { PacketReason } from '#lib/api';

	export interface ResumeChoiceProps {
		/** The chosen resume; `null` once it has been deleted. */
		resumeId: number | null;
		/** The resume's name when the packet was prepared. */
		name: string;
		reasons: PacketReason[];
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import CitationList from '#lib/components/postings/CitationList.svelte';
	import { Badge, Link } from '#lib/components/ui';

	/** The resume a packet picked, linking to it (or marked deleted), and the cited reasons. */
	let { resumeId, name, reasons }: ResumeChoiceProps = $props();

	const href = $derived(
		resumeId === null ? null : resolve('/resumes/[id]', { id: String(resumeId) })
	);
</script>

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap items-center gap-2">
		{#if href}
			<Link {href} class="text-sm font-semibold break-words">{name}</Link>
		{:else}
			<span class="text-sm font-semibold break-words text-fg">{name}</span>
			<Badge tone="neutral">Deleted resume</Badge>
		{/if}
	</div>
	{#if reasons.length > 0}
		<ul class="flex flex-col gap-4" aria-label="Why this resume">
			{#each reasons as reason, i (i)}
				<li class="flex flex-col gap-2">
					<p class="text-sm break-words text-fg">{reason.text}</p>
					<CitationList citations={reason.citations} />
				</li>
			{/each}
		</ul>
	{/if}
</div>
