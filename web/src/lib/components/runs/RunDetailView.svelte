<script lang="ts">
	import { SvelteSet } from 'svelte/reactivity';
	import type { RunDetail } from '#lib/api';
	import { Card, JsonView } from '#lib/components/ui';
	import MemoriesUsed from './MemoriesUsed.svelte';
	import RunStatusBadge from './RunStatusBadge.svelte';
	import RunSummary from './RunSummary.svelte';
	import StepItem from './StepItem.svelte';

	let { run }: { run: RunDetail } = $props();

	// Keyed by id, so what's open stays open when a live update replaces the run.
	const openSteps = new SvelteSet<number>();
	const openCalls = new SvelteSet<number>();

	function toggle(set: SvelteSet<number>, id: number) {
		if (set.has(id)) set.delete(id);
		else set.add(id);
	}

	let now = $state(Date.now());

	// Ticks once a second while the run is running, so its duration counts up.
	$effect(() => {
		if (run.status !== 'running') return;
		now = Date.now();
		const timer = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(timer);
	});
</script>

<div class="flex flex-col gap-8">
	<header class="flex flex-wrap items-center gap-2">
		<h1 class="text-2xl font-semibold break-words text-fg">{run.workflow}</h1>
		<span class="text-2xl text-fg-muted tabular-nums">#{run.id}</span>
		<RunStatusBadge status={run.status} />
	</header>

	<RunSummary {run} {now} />

	<div class="grid gap-4 md:grid-cols-2">
		<Card>
			<section class="flex min-w-0 flex-col gap-2" aria-labelledby="run-input-heading">
				<h2 id="run-input-heading" class="text-sm font-semibold text-fg">Input</h2>
				<JsonView value={run.input} label="Run input" />
			</section>
		</Card>
		<Card>
			<section class="flex min-w-0 flex-col gap-2" aria-labelledby="run-output-heading">
				<h2 id="run-output-heading" class="text-sm font-semibold text-fg">Output</h2>
				<JsonView value={run.output} label="Run output" empty="No output yet" />
			</section>
		</Card>
	</div>

	<section class="flex flex-col gap-4" aria-labelledby="steps-heading">
		<h2 id="steps-heading" class="text-lg font-semibold text-fg">Steps</h2>
		<ol class="flex flex-col gap-4" aria-label="Steps">
			{#each run.steps as step (step.id)}
				<li>
					<StepItem
						{step}
						{now}
						showIo={openSteps.has(step.id)}
						ontoggleio={() => toggle(openSteps, step.id)}
						expandedCalls={openCalls}
						ontogglecall={(id) => toggle(openCalls, id)}
					/>
				</li>
			{/each}
		</ol>
	</section>

	<MemoriesUsed />
</div>
