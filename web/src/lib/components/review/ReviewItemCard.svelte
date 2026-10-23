<script lang="ts" module>
	import type { ReviewItem } from '#lib/api';

	export interface ReviewItemCardProps {
		item: ReviewItem;
		/** A decision is in flight; the buttons are disabled. */
		busy?: boolean;
		ondismiss: () => void;
		/** Applies the item. Only email, fit-score and packet items with a readable payload offer it. */
		onaccept?: () => void;
	}
</script>

<script lang="ts">
	import { slide } from 'svelte/transition';
	import { Badge, Button, Card, JsonView, type BadgeTone } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import { describeEmailReview } from '#lib/review/email.js';
	import { describeFitReview } from '#lib/review/fit.js';
	import { describePacketReview } from '#lib/review/packet.js';
	import EmailReviewSummary from './EmailReviewSummary.svelte';
	import FitReviewSummary from './FitReviewSummary.svelte';
	import ItemMeta from './ItemMeta.svelte';
	import PacketReviewSummary from './PacketReviewSummary.svelte';

	let { item, busy = false, ondismiss, onaccept }: ReviewItemCardProps = $props();

	const confidence = $derived(Math.round(item.confidence * 100));
	const email = $derived(describeEmailReview(item));
	const fit = $derived(describeFitReview(item));
	const packet = $derived(describePacketReview(item));
	const summary = $derived(email ?? fit ?? packet);
	const badge = $derived<{ label: string; tone: BadgeTone }>(
		summary?.badge ?? { label: 'Low confidence', tone: 'warning' }
	);
	let showRaw = $state(false);
	const rawId = $derived(`review-item-${item.id}-raw`);
</script>

<Card class="flex flex-col gap-4" data-review-item={item.id}>
	<div class="flex flex-col gap-2">
		<div class="flex flex-wrap items-center gap-2">
			<Badge tone={badge.tone}>{badge.label}</Badge>
			<span class="text-xs text-fg-muted tabular-nums">{confidence}% confident</span>
		</div>
		<ItemMeta name={item.kind} runId={item.run_id} createdAt={item.created_at} />
	</div>
	{#if summary}
		{#if email}
			<EmailReviewSummary summary={email} />
		{:else if fit}
			<FitReviewSummary summary={fit} />
		{:else if packet}
			<PacketReviewSummary summary={packet} />
		{/if}
		<div class="flex flex-col gap-2">
			<Button
				variant="ghost"
				size="sm"
				class="self-start"
				aria-expanded={showRaw}
				aria-controls={rawId}
				onclick={() => (showRaw = !showRaw)}
			>
				{showRaw ? 'Hide raw payload' : 'Show raw payload'}
			</Button>
			{#if showRaw}
				<div id={rawId} transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}>
					<JsonView value={item.payload} label="Result for {item.kind}" />
				</div>
			{/if}
		</div>
	{:else}
		<JsonView value={item.payload} label="Result for {item.kind}" />
	{/if}
	<div class="flex justify-end gap-2">
		<Button variant="secondary" disabled={busy} onclick={ondismiss}>Dismiss</Button>
		{#if summary && onaccept}
			<Button disabled={busy} onclick={onaccept}>Apply</Button>
		{/if}
	</div>
</Card>
