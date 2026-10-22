<script lang="ts" module>
	import type { ApplicationEvent } from '#lib/api';

	export interface TimelineProps {
		/** Newest first. */
		events: ApplicationEvent[];
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { motionDuration } from '#lib/motion.js';
	import TimelineItem from './TimelineItem.svelte';

	let { events }: TimelineProps = $props();
</script>

{#if events.length === 0}
	<p class="text-sm text-fg-muted">No activity yet</p>
{:else}
	<ol class="flex flex-col gap-4" aria-label="Timeline">
		{#each events as event (event.id)}
			<li class="border-l border-border pl-4" in:fade={{ duration: motionDuration('base') }}>
				<TimelineItem {event} />
			</li>
		{/each}
	</ol>
{/if}
