<script lang="ts">
	import type { Run } from '#lib/api';
	import { Stat } from '#lib/components/ui';
	import { formatCost, formatElapsed, formatTokens } from '#lib/runs/format.js';
	import { runStatusBadge } from '#lib/runs/statuses.js';
	import RunError from './RunError.svelte';

	let {
		run,
		now
	}: {
		run: Run;
		/** The clock a running run's duration counts up to. */
		now: number;
	} = $props();

	const status = $derived(runStatusBadge[run.status]);
</script>

<section class="flex flex-col gap-4" aria-label="Summary">
	<div class="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
		<Stat label="Status" value={status.label} tone={status.tone} />
		<Stat label="Duration" value={formatElapsed(run, now)} />
		<Stat label="Tokens in" value={formatTokens(run.input_tokens)} />
		<Stat label="Tokens out" value={formatTokens(run.output_tokens)} />
		<Stat label="Cost" value={formatCost(run.cost_usd)} />
	</div>
	{#if run.status === 'failed' && run.error}
		<RunError message={run.error} />
	{/if}
</section>
