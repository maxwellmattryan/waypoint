<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { Source } from '#lib/api';

	export interface CompanyRowProps {
		company: {
			name: string;
			source: Source | '';
			board: string;
			reason?: string | null;
			/** `false` adds the Unverified board badge; omitted for a draft. */
			verified?: boolean;
		};
		/** Buttons at the end of the row, e.g. the editor's Edit and Remove. */
		actions?: Snippet;
	}
</script>

<script lang="ts">
	import { Badge } from '#lib/components/ui';

	/** One target company: its name, `source/board`, its reason and whether its board is known. */
	let { company, actions }: CompanyRowProps = $props();
</script>

<div class="flex flex-col gap-2 py-2 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
	<div class="flex min-w-0 flex-col gap-1">
		<p class="flex flex-wrap items-center gap-2">
			<span class="text-sm font-semibold break-words text-fg">{company.name}</span>
			<span class="font-mono text-xs break-all text-fg-muted">{company.source}/{company.board}</span>
			{#if company.verified === false}
				<Badge tone="warning" title="Waypoint hasn't read this board yet; the source step checks it">
					Unverified board
				</Badge>
			{/if}
		</p>
		{#if company.reason}
			<p class="text-sm break-words text-fg-muted">{company.reason}</p>
		{/if}
	</div>
	{#if actions}
		<div class="flex shrink-0 gap-2">{@render actions()}</div>
	{/if}
</div>
