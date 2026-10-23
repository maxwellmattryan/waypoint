<script lang="ts">
	import { flip } from 'svelte/animate';
	import type { PlanStep } from '#lib/api';
	import { easings, motionDuration } from '#lib/motion.js';
	import PlanStepItem from './PlanStepItem.svelte';

	/** A plan's steps, in order. */
	let { steps }: { steps: PlanStep[] } = $props();
</script>

<ol class="flex flex-col divide-y divide-border" aria-label="Steps">
	{#each steps as step (step.id)}
		<li animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}>
			<PlanStepItem {step} />
		</li>
	{/each}
</ol>
