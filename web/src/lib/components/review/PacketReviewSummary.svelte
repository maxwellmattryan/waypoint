<script lang="ts" module>
	import type { PacketReview } from '#lib/review/packet.js';

	export interface PacketReviewSummaryProps {
		summary: PacketReview;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import CitationList from '#lib/components/postings/CitationList.svelte';
	import { KeyValue, Link, type KeyValueItem } from '#lib/components/ui';

	/**
	 * A packet review item, readable: the posting, the chosen resume, the missing answers and the
	 * drafted answers with their citations.
	 */
	let { summary }: PacketReviewSummaryProps = $props();

	const postingHref = $derived(resolve('/postings/[id]', { id: String(summary.postingId) }));

	const items = $derived.by(() => {
		const list: KeyValueItem[] = [
			{ key: 'Posting', value: posting },
			{ key: 'Resume', value: summary.resumeName }
		];
		if (summary.missing.length > 0) list.push({ key: 'Missing answers', value: missing });
		if (summary.answers.length > 0) list.push({ key: 'Answers', value: answers });
		return list;
	});
</script>

{#snippet posting()}
	<Link href={postingHref}>{summary.company} — {summary.title}</Link>
{/snippet}

{#snippet missing()}
	<ul class="flex list-disc flex-col gap-1 pl-4" aria-label="Missing answers">
		{#each summary.missing as question, i (i)}
			<li class="break-words">{question}</li>
		{/each}
	</ul>
{/snippet}

{#snippet answers()}
	<ol class="flex flex-col gap-4">
		{#each summary.answers as answer (answer.key)}
			<li class="flex flex-col gap-2">
				<p class="text-sm font-semibold break-words text-fg">{answer.question}</p>
				<p class="text-sm break-words whitespace-pre-wrap text-fg">{answer.answer}</p>
				{#if answer.citations.length > 0}
					<CitationList citations={answer.citations} />
				{/if}
			</li>
		{/each}
	</ol>
{/snippet}

<KeyValue {items} />
