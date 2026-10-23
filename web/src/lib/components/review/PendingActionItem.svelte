<script lang="ts" module>
	import type { PendingAction } from '#lib/api';

	export interface PendingActionItemProps {
		action: PendingAction;
		/** A decision on this item is in flight; its buttons are disabled. */
		busy?: boolean;
		onapprove: () => void;
		onedit: () => void;
		onreject: () => void;
	}
</script>

<script lang="ts">
	import PermissionBadge from '#lib/components/runs/PermissionBadge.svelte';
	import { Badge, Button, Card, JsonView } from '#lib/components/ui';
	import ItemMeta from './ItemMeta.svelte';

	let { action, busy = false, onapprove, onedit, onreject }: PendingActionItemProps = $props();
</script>

<Card class="flex flex-col gap-4" data-pending-action={action.id}>
	<div class="flex flex-col gap-2">
		<div class="flex flex-wrap items-center gap-2">
			<Badge tone="warning">Needs approval</Badge>
			<!-- Only `external` tools reach the approval gate. -->
			<PermissionBadge permission="external" />
		</div>
		<ItemMeta name={action.tool} runId={action.run_id} createdAt={action.created_at} />
	</div>
	<JsonView value={action.input} label="Requested input for {action.tool}" />
	<div class="flex flex-wrap justify-end gap-2">
		<Button variant="danger" disabled={busy} onclick={onreject}>Reject</Button>
		<Button variant="secondary" disabled={busy} onclick={onedit}>Edit</Button>
		<Button disabled={busy} onclick={onapprove}>Approve</Button>
	</div>
</Card>
