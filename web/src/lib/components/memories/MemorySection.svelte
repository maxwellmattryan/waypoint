<script lang="ts" module>
	import type { Memory } from '#lib/api';
	import type { MemoryItemProps } from './MemoryItem.svelte';

	export type MemorySectionProps = Omit<MemoryItemProps, 'memory' | 'highlighted'> & {
		/** The section heading, e.g. "Answer bank". */
		heading: string;
		memories: Memory[];
		/** The id of the memory to highlight, if it's in this section. */
		highlighted?: number | null;
	};
</script>

<script lang="ts">
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
	import MemoryItem from './MemoryItem.svelte';

	let { heading, memories, highlighted = null, ...actions }: MemorySectionProps = $props();

	const headingId = $props.id();
</script>

<section class="flex flex-col gap-4" aria-labelledby={headingId}>
	<h2 id={headingId} class="text-lg font-semibold text-fg">{heading}</h2>
	<ul class="flex flex-col gap-2" aria-label={heading}>
		{#each memories as memory (memory.id)}
			<li
				animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
				transition:fade={{ duration: motionDuration('fast') }}
			>
				<MemoryItem {memory} highlighted={memory.id === highlighted} {...actions} />
			</li>
		{/each}
	</ul>
</section>
