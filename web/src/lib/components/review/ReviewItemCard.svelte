<script lang="ts" module>
	import type { ReviewItem } from '#lib/api';

	export interface ReviewItemCardProps {
		item: ReviewItem;
		/** A dismissal is in flight; the button is disabled. */
		busy?: boolean;
		ondismiss: () => void;
	}
</script>

<script lang="ts">
	import { Badge, Button, Card, JsonView } from '#lib/components/ui';
	import ItemMeta from './ItemMeta.svelte';

	let { item, busy = false, ondismiss }: ReviewItemCardProps = $props();

	const confidence = $derived(Math.round(item.confidence * 100));
</script>

<Card class="flex flex-col gap-4" data-review-item={item.id}>
	<div class="flex flex-col gap-2">
		<div class="flex flex-wrap items-center gap-2">
			<Badge tone="warning">Low confidence</Badge>
			<span class="text-xs text-fg-muted tabular-nums">{confidence}% confident</span>
		</div>
		<ItemMeta name={item.kind} runId={item.run_id} createdAt={item.created_at} />
	</div>
	<JsonView value={item.payload} label="Result for {item.kind}" />
	<div class="flex justify-end">
		<Button variant="secondary" disabled={busy} onclick={ondismiss}>Dismiss</Button>
	</div>
</Card>
