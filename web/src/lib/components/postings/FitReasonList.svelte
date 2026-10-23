<script lang="ts" module>
	import type { FitReason } from '#lib/api';

	export interface FitReasonListProps {
		reasons: FitReason[];
	}
</script>

<script lang="ts">
	import { Link } from '#lib/components/ui';
	import { memoryHref } from '#lib/memories/link.js';
	import { TOPIC_LABELS } from '#lib/memories/topics.js';

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
						<ul class="flex flex-col gap-2 border-l border-border pl-4" aria-label="Evidence">
							{#each reason.citations as citation, j (j)}
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
					</li>
				{/each}
			</ul>
		</section>
	{/each}
</div>
