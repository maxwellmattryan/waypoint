<script lang="ts">
	import type { PlanStep } from '#lib/api';
	import { STEP_LABELS, stepSummary } from '#lib/goals/format.js';
	import PlanStepStatusBadge from './PlanStepStatusBadge.svelte';

	/** One plan step: its name, status, a one-line summary and where it was kept from. */
	let { step }: { step: PlanStep } = $props();
</script>

<div class="flex flex-col gap-1 py-2" data-step={step.kind}>
	<div class="flex flex-wrap items-center gap-2">
		<span class="text-xs text-fg-subtle tabular-nums">{step.ordinal + 1}</span>
		<span class="text-sm font-semibold text-fg">{STEP_LABELS[step.kind]}</span>
		<PlanStepStatusBadge status={step.status} />
		{#if step.kept_from_version !== null}
			<span class="text-xs text-fg-muted">kept from version {step.kept_from_version}</span>
		{/if}
	</div>
	<p
		class={[
			'text-sm break-words whitespace-pre-wrap',
			step.status === 'failed' ? 'text-danger' : 'text-fg-muted'
		]}
	>
		{stepSummary(step)}
	</p>
</div>
