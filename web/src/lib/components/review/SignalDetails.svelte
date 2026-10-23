<script lang="ts">
	import type { MemorySignal } from '#lib/api';
	import { Badge } from '#lib/components/ui';
	import { compactJson, signalLabel } from '#lib/review/signals.js';
	import ItemMeta from './ItemMeta.svelte';

	/** One signal's details: its decision badge, subject, run and time, reason and changes. */
	let { signal }: { signal: MemorySignal } = $props();

	const badge = $derived(signalLabel(signal));
	const changes = $derived(signal.kind === 'approval' ? (signal.changes ?? []) : []);
</script>

{#snippet value(label: string, json: string | null)}
	<span class="flex min-w-0 items-baseline gap-2">
		<span class="shrink-0 text-xs text-fg-muted">{label}</span>
		{#if json === null}
			<span class="text-xs text-fg-muted" aria-label="none">–</span>
		{:else}
			<code class="truncate font-mono text-xs text-fg" title={json}>{json}</code>
		{/if}
	</span>
{/snippet}

<div class="flex flex-col gap-4">
	<div class="flex flex-col gap-2">
		<div class="flex flex-wrap items-center gap-2">
			<Badge tone={badge.tone}>{badge.label}</Badge>
		</div>
		<ItemMeta name={signal.subject} runId={signal.run_id} createdAt={signal.created_at} />
	</div>
	{#if signal.kind === 'dismissal' && signal.reason}
		<blockquote class="border-l border-border pl-4 text-sm break-words text-fg">
			<span class="sr-only">Reason: </span>{signal.reason}
		</blockquote>
	{/if}
	{#if changes.length > 0}
		<ul
			class="flex flex-col divide-y divide-border rounded-md border border-border"
			aria-label="Changes"
		>
			{#each changes as change, i (i)}
				<li
					class="grid grid-cols-1 gap-2 px-4 py-2 sm:grid-cols-[auto_minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)] sm:items-baseline sm:gap-4"
					data-change={change.op}
				>
					<span class="font-mono text-xs font-semibold text-fg-muted uppercase">{change.op}</span>
					<code class="truncate font-mono text-xs text-fg" title={change.path || '/'}>
						{change.path === '' ? '(whole input)' : change.path}
					</code>
					{@render value(
						'Before',
						change.op === 'add' ? null : compactJson(change.before)
					)}
					{@render value('After', change.op === 'remove' ? null : compactJson(change.after))}
				</li>
			{/each}
		</ul>
	{/if}
</div>
