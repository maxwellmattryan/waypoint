<script lang="ts">
	import { slide } from 'svelte/transition';
	import type { ToolCall } from '#lib/api';
	import { Button, JsonView, KeyValue } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import { formatCost, formatMs, formatTokenPair } from '#lib/runs/format.js';
	import PermissionBadge from './PermissionBadge.svelte';
	import ToolCallStatusBadge from './ToolCallStatusBadge.svelte';

	let {
		call,
		expanded,
		ontoggle
	}: { call: ToolCall; expanded: boolean; ontoggle: () => void } = $props();

	const detailsId = $derived(`tool-call-${call.id}-details`);
</script>

<div class="flex flex-col gap-2 rounded-md border border-border p-3" data-tool-call={call.id}>
	<div class="flex flex-wrap items-center justify-between gap-2">
		<div class="flex min-w-0 flex-wrap items-center gap-2">
			<p class="truncate font-mono text-sm font-medium text-fg">{call.tool}</p>
			<PermissionBadge permission={call.permission} />
			<ToolCallStatusBadge status={call.status} />
		</div>
		<Button
			variant="ghost"
			size="sm"
			aria-expanded={expanded}
			aria-controls={detailsId}
			onclick={ontoggle}
		>
			{expanded ? 'Hide details' : 'Show details'}
		</Button>
	</div>
	<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted tabular-nums">
		<span>{formatMs(call.duration_ms)}</span>
		<span>{formatTokenPair(call.input_tokens, call.output_tokens)}</span>
		<span>{formatCost(call.cost_usd)}</span>
	</p>
	{#if expanded}
		<div
			id={detailsId}
			class="pt-2"
			transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}
		>
			{#snippet input()}
				<JsonView value={call.input} label="{call.tool} input" />
			{/snippet}
			{#snippet output()}
				<JsonView value={call.output} label="{call.tool} output" empty="No output" />
			{/snippet}
			{#snippet error()}
				<JsonView value={call.error} label="{call.tool} error" empty="No error" />
			{/snippet}
			<KeyValue
				items={[
					{ key: 'Input', value: input },
					{ key: 'Output', value: output },
					{ key: 'Error', value: error }
				]}
			/>
		</div>
	{/if}
</div>
