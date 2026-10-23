<script lang="ts" module>
	import type { PacketAnswer } from '#lib/api';

	export interface PacketAnswerListProps {
		answers: PacketAnswer[];
	}
</script>

<script lang="ts">
	import CitationList from '#lib/components/postings/CitationList.svelte';
	import { Badge } from '#lib/components/ui';

	/**
	 * A packet's answers: each question with its drafted answer as plain, pre-wrapped text and
	 * its citations, or a `Missing` badge when nothing grounded an answer.
	 */
	let { answers }: PacketAnswerListProps = $props();
</script>

<ol class="flex flex-col gap-6">
	{#each answers as answer (answer.key)}
		<li class="flex flex-col gap-2" data-answer={answer.key}>
			<div class="flex flex-wrap items-center gap-2">
				<h3 class="text-sm font-semibold break-words text-fg">{answer.question}</h3>
				{#if answer.answer === null}
					<Badge tone="warning">Missing</Badge>
				{/if}
			</div>
			{#if answer.answer === null}
				<p class="text-sm text-fg-muted">Waypoint had nothing to draft this from.</p>
			{:else}
				<p class="text-sm leading-6 break-words whitespace-pre-wrap text-fg">{answer.answer}</p>
				{#if answer.citations.length > 0}
					<CitationList citations={answer.citations} />
				{/if}
			{/if}
		</li>
	{/each}
</ol>
