<script lang="ts">
	import { resolve } from '$app/paths';
	import type { Run } from '#lib/api';
	import { formatDateTime } from '#lib/applications/format.js';
	import { Card } from '#lib/components/ui';
	import { formatCost, formatElapsed, formatTokenPair } from '#lib/runs/format.js';
	import RunStatusBadge from './RunStatusBadge.svelte';

	let { run }: { run: Run } = $props();

	const href = $derived(resolve('/runs/[id]', { id: String(run.id) }));
</script>

<Card {href}>
	<div class="flex items-start justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<p class="truncate text-sm text-fg">
				<span class="font-semibold">{run.workflow}</span>
				<span class="text-fg-muted">#{run.id}</span>
			</p>
			<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted tabular-nums">
				<span>{formatDateTime(run.created_at)}</span>
				<span>{formatElapsed(run)}</span>
				<span>{formatTokenPair(run.input_tokens, run.output_tokens)}</span>
				<span>{formatCost(run.cost_usd)}</span>
			</p>
		</div>
		<RunStatusBadge status={run.status} />
	</div>
</Card>
