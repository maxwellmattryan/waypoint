<script lang="ts">
	import { slide } from 'svelte/transition';
	import type { RunStepDetail } from '#lib/api';
	import { Button, Card, JsonView } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import { formatCost, formatElapsed, formatTokenPair } from '#lib/runs/format.js';
	import RunError from './RunError.svelte';
	import StepStatusBadge from './StepStatusBadge.svelte';
	import ToolCallItem from './ToolCallItem.svelte';

	let {
		step,
		now,
		showIo,
		ontoggleio,
		expandedCalls,
		ontogglecall
	}: {
		step: RunStepDetail;
		/** The clock for a running step's duration. */
		now: number;
		/** Whether the step's input and output are shown. */
		showIo: boolean;
		ontoggleio: () => void;
		/** Tool-call ids whose details are shown. */
		expandedCalls: ReadonlySet<number>;
		ontogglecall: (id: number) => void;
	} = $props();

	const ioId = $derived(`step-${step.id}-io`);
</script>

<Card>
	<article class="flex flex-col gap-4" aria-labelledby="step-{step.id}-name" data-step={step.id}>
		<div class="flex flex-wrap items-start justify-between gap-2">
			<div class="flex min-w-0 flex-col gap-1">
				<div class="flex flex-wrap items-center gap-2">
					<h3 id="step-{step.id}-name" class="truncate font-mono text-sm font-semibold text-fg">
						{step.name}
					</h3>
					<StepStatusBadge status={step.status} />
					{#if step.attempts > 1}
						<span class="text-xs text-fg-muted">{step.attempts} attempts</span>
					{/if}
				</div>
				<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted tabular-nums">
					<span>{formatElapsed(step, now)}</span>
					<span>{formatTokenPair(step.input_tokens, step.output_tokens)}</span>
					<span>{formatCost(step.cost_usd)}</span>
				</p>
			</div>
			<Button
				variant="ghost"
				size="sm"
				aria-expanded={showIo}
				aria-controls={ioId}
				onclick={ontoggleio}
			>
				{showIo ? 'Hide input and output' : 'Show input and output'}
			</Button>
		</div>

		{#if step.error}
			<RunError message={step.error} />
		{/if}

		{#if showIo}
			<div
				id={ioId}
				class="grid gap-4 md:grid-cols-2"
				transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}
			>
				<div class="flex min-w-0 flex-col gap-2">
					<p class="text-xs font-medium text-fg-muted">Input</p>
					<JsonView value={step.input} label="{step.name} input" empty="Not started" />
				</div>
				<div class="flex min-w-0 flex-col gap-2">
					<p class="text-xs font-medium text-fg-muted">Output</p>
					<JsonView value={step.output} label="{step.name} output" empty="No output yet" />
				</div>
			</div>
		{/if}

		{#if step.tool_calls.length > 0}
			<div class="flex flex-col gap-2">
				<p class="text-xs font-medium text-fg-muted">
					Tool calls ({step.tool_calls.length})
				</p>
				<ul class="flex flex-col gap-2" aria-label="{step.name} tool calls">
					{#each step.tool_calls as call (call.id)}
						<li>
							<ToolCallItem
								{call}
								expanded={expandedCalls.has(call.id)}
								ontoggle={() => ontogglecall(call.id)}
							/>
						</li>
					{/each}
				</ul>
			</div>
		{/if}
	</article>
</Card>
