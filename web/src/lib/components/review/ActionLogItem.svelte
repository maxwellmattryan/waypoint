<script lang="ts" module>
	import type { AgentAction } from '#lib/api';

	export interface ActionLogItemProps {
		action: AgentAction;
		/** An undo is in flight; the button is disabled. */
		busy?: boolean;
		onundo: () => void;
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { formatDateTime } from '#lib/applications/format.js';
	import PermissionBadge from '#lib/components/runs/PermissionBadge.svelte';
	import { Badge, Button, Card, Tooltip } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import ItemMeta from './ItemMeta.svelte';

	let { action, busy = false, onundo }: ActionLogItemProps = $props();

	const mode = $derived(
		action.undone_at !== null ? 'undone' : action.undo_data === null ? 'permanent' : 'undoable'
	);
</script>

<Card class="flex flex-wrap items-center justify-between gap-4" data-action={action.id}>
	<div class="flex min-w-0 flex-col gap-2">
		<div class="flex flex-wrap items-center gap-2">
			<!-- Only `write` tools are logged. -->
			<PermissionBadge permission="write" />
		</div>
		<ItemMeta name={action.tool} runId={action.run_id} createdAt={action.created_at} />
	</div>
	{#key mode}
		<div
			class="flex items-center gap-2"
			in:fade={{ duration: motionDuration('base'), easing: easings.enter }}
		>
			{#if action.undone_at !== null}
				<Badge>Undone</Badge>
				<span class="text-xs text-fg-muted tabular-nums">
					Undone {formatDateTime(action.undone_at)}
				</span>
			{:else if action.undo_data === null}
				<Tooltip content="This tool didn't record how to undo it.">
					{#snippet children(trigger)}
						<Badge tabindex={0} {...trigger}>Can't undo</Badge>
					{/snippet}
				</Tooltip>
			{:else}
				<Button variant="secondary" disabled={busy} onclick={onundo}>Undo</Button>
			{/if}
		</div>
	{/key}
</Card>
