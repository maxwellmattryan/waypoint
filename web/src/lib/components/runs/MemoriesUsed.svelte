<script lang="ts">
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import type { RunMemory } from '#lib/api';
	import { Card, EmptyState, Link } from '#lib/components/ui';
	import { memoryHref } from '#lib/memories/link.js';
	import { TOPIC_LABELS } from '#lib/memories/topics.js';
	import { easings, motionDuration } from '#lib/motion.js';

	/** The run's memories, in the API's order (by step, then retrieval). */
	let { memories }: { memories: RunMemory[] } = $props();
</script>

<!-- Each entry shows the snapshot the step saw, so an edited or deleted memory still reads as used. -->
<section class="flex flex-col gap-4" aria-labelledby="memories-heading">
	<h2 id="memories-heading" class="text-lg font-semibold text-fg">Memories used</h2>
	{#if memories.length === 0}
		<EmptyState
			title="No memories used"
			description="This run didn't use any memories."
			class="rounded-lg border border-dashed border-border"
		/>
	{:else}
		<ul class="flex flex-col gap-2" aria-label="Memories used">
			{#each memories as memory (memory.id)}
				<li
					animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
					transition:fade={{ duration: motionDuration('fast') }}
				>
					<Card>
						<div class="flex flex-col gap-2">
							<p class="text-xs font-medium text-fg-muted">{TOPIC_LABELS[memory.topic]}</p>
							<p class="text-sm break-words whitespace-pre-line text-fg">{memory.content}</p>
							<p class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-fg-muted">
								<span>Used in {memory.step}</span>
								{#if memory.memory_id !== null}
									<Link href={memoryHref(memory.memory_id)}>View memory</Link>
								{:else}
									<span>Deleted</span>
								{/if}
							</p>
						</div>
					</Card>
				</li>
			{/each}
		</ul>
	{/if}
</section>
