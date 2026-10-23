<script lang="ts" module>
	import type { FitCitation } from '#lib/api';

	export interface CitationListProps {
		citations: FitCitation[];
	}
</script>

<script lang="ts">
	import { Link } from '#lib/components/ui';
	import { memoryHref } from '#lib/memories/link.js';
	import { TOPIC_LABELS } from '#lib/memories/topics.js';

	/**
	 * Checked citations as an "Evidence" list: a posting excerpt as a quote, a memory as its
	 * topic and snapshot with View memory, or "Deleted memory" once it is gone. All plain text.
	 */
	let { citations }: CitationListProps = $props();
</script>

<ul class="flex flex-col gap-2 border-l border-border pl-4" aria-label="Evidence">
	{#each citations as citation, j (j)}
		<li class="flex flex-col gap-1 text-sm">
			{#if citation.source === 'posting'}
				<blockquote class="break-words text-fg-muted italic">
					“{citation.text}”
				</blockquote>
				<span class="text-xs text-fg-subtle">From the posting</span>
			{:else}
				<p class="break-words text-fg-muted">
					{#if citation.topic}
						<span class="font-medium text-fg">{TOPIC_LABELS[citation.topic]}:</span>
					{/if}
					{citation.text}
				</p>
				{#if citation.memory_id !== null}
					<Link href={memoryHref(citation.memory_id)} class="self-start text-xs">
						View memory
					</Link>
				{:else}
					<span class="text-xs text-fg-subtle">Deleted memory</span>
				{/if}
			{/if}
		</li>
	{/each}
</ul>
