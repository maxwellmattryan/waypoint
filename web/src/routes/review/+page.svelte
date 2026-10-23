<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { createApiClient, type PendingAction, type ReviewItem } from '#lib/api';
	import type { Result } from '#lib/applications/api.js';
	import ActionLogItem from '#lib/components/review/ActionLogItem.svelte';
	import DismissReviewItemDialog from '#lib/components/review/DismissReviewItemDialog.svelte';
	import EditActionDialog from '#lib/components/review/EditActionDialog.svelte';
	import PendingActionItem from '#lib/components/review/PendingActionItem.svelte';
	import RejectActionDialog from '#lib/components/review/RejectActionDialog.svelte';
	import ReviewItemCard from '#lib/components/review/ReviewItemCard.svelte';
	import SignalItem from '#lib/components/review/SignalItem.svelte';
	import { Badge, Button, EmptyState, Link, Skeleton, Tabs } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { easings, motionDuration } from '#lib/motion.js';
	import {
		ActionLogStore,
		agentsPaused,
		QueueStore,
		queueKey,
		SignalLogStore
	} from '#lib/review/review.svelte.js';

	const client = createApiClient();
	const dashboard = getDashboard();
	const paused = $derived(agentsPaused(dashboard.killSwitch));
	const queue = new QueueStore({ client, paused: () => paused });
	const log = new ActionLogStore({ client });
	const signals = new SignalLogStore({ client });

	let tab = $state('queue');
	const queueLabel = $derived(queue.items.length > 0 ? `Queue (${queue.items.length})` : 'Queue');

	let editing = $state<PendingAction | null>(null);
	let editOpen = $state(false);
	let rejecting = $state<PendingAction | null>(null);
	let rejectOpen = $state(false);
	let dismissing = $state<ReviewItem | null>(null);
	let dismissOpen = $state(false);

	const dashboardHref = resolve('/');

	function edit(action: PendingAction) {
		editing = action;
		editOpen = true;
	}

	function confirmReject(action: PendingAction) {
		rejecting = action;
		rejectOpen = true;
	}

	function confirmDismiss(item: ReviewItem) {
		dismissing = item;
		dismissOpen = true;
	}

	/** A queue decision; once it's stored, its learning signal shows in the Signals tab. */
	async function decided<T>(decision: Promise<Result<T>>): Promise<Result<T>> {
		const result = await decision;
		if (result.kind === 'ok') void signals.refresh();
		return result;
	}

	onMount(() => {
		void queue.load();
		void log.load();
		void signals.load();
		return dashboard.subscribe((event) => {
			queue.handle(event);
			log.handle(event);
			signals.handle(event);
		});
	});
</script>

<svelte:head>
	<title>Review · Waypoint</title>
</svelte:head>

{#snippet skeleton(label: string)}
	<div class="flex flex-col gap-2" aria-busy="true" aria-label={label}>
		{#each [0, 1, 2] as row (row)}
			<Skeleton class="h-24" />
		{/each}
	</div>
{/snippet}

<div class="flex flex-col gap-8">
	<div class="flex flex-col gap-1">
		<h1 class="text-2xl font-semibold text-fg">Review</h1>
		<p class="text-sm text-fg-muted">
			Actions waiting for your approval, results an agent wasn't sure of, every change agents
			made, and what Waypoint learns from your decisions.
		</p>
	</div>

	{#if paused}
		<div
			role="status"
			class="flex items-start gap-2 rounded-lg border border-border bg-surface p-4 text-sm text-fg"
			transition:fade={{ duration: motionDuration('base'), easing: easings.standard }}
		>
			<Badge tone="warning">Paused</Badge>
			<p class="py-1">
				Agents are paused. Runs you approve wait as Paused until you resume agents on the
				dashboard. <Link href={dashboardHref}>Dashboard</Link>
			</p>
		</div>
	{/if}

	<Tabs
		label="Review"
		items={[
			{ value: 'queue', label: queueLabel },
			{ value: 'log', label: 'Action log' },
			{ value: 'signals', label: 'Signals' }
		]}
		bind:value={tab}
	>
		{#snippet children(current)}
			{#if current === 'queue'}
				{#if queue.state.kind === 'loading'}
					{@render skeleton('Loading review queue')}
				{:else if queue.state.kind === 'error'}
					<div in:fade={{ duration: motionDuration('base') }}>
						<EmptyState
							title="Couldn't load the review queue"
							description="Check that the API is running, then try again."
						>
							{#snippet action()}
								<Button variant="secondary" onclick={() => queue.load()}>Try again</Button>
							{/snippet}
						</EmptyState>
					</div>
				{:else if queue.items.length === 0}
					<div in:fade={{ duration: motionDuration('base') }}>
						<EmptyState
							title="Nothing to review"
							description="Actions waiting for your approval and results an agent wasn't sure of show here."
						/>
					</div>
				{:else}
					<ul class="flex flex-col gap-4" aria-label="Review queue">
						{#each queue.items as entry (queueKey(entry))}
							{@const busy = queue.busy.has(queueKey(entry))}
							<li
								animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
								in:fade={{ duration: motionDuration('base'), easing: easings.enter }}
								out:fade={{ duration: motionDuration('base'), easing: easings.exit }}
							>
								{#if entry.kind === 'action'}
									{@const action = entry.action}
									<PendingActionItem
										{action}
										{busy}
										onapprove={() => decided(queue.approve(action.id))}
										onedit={() => edit(action)}
										onreject={() => confirmReject(action)}
									/>
								{:else}
									{@const item = entry.item}
									<ReviewItemCard {item} {busy} ondismiss={() => confirmDismiss(item)} />
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			{:else if current === 'signals'}
				{#if signals.state.kind === 'loading'}
					{@render skeleton('Loading signals')}
				{:else if signals.state.kind === 'error'}
					<div in:fade={{ duration: motionDuration('base') }}>
						<EmptyState
							title="Couldn't load the signals"
							description="Check that the API is running, then try again."
						>
							{#snippet action()}
								<Button variant="secondary" onclick={() => signals.load()}>Try again</Button>
							{/snippet}
						</EmptyState>
					</div>
				{:else if signals.signals.length === 0}
					<div in:fade={{ duration: motionDuration('base') }}>
						<EmptyState
							title="No signals yet"
							description="Your approvals, edits, rejections and dismissals show here. Waypoint learns from them."
						/>
					</div>
				{:else}
					<div class="flex flex-col gap-4">
						<ul class="flex flex-col gap-2" aria-label="Signals">
							{#each signals.signals as signal (signal.id)}
								<li
									animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
									in:fade={{ duration: motionDuration('base'), easing: easings.enter }}
								>
									<SignalItem {signal} />
								</li>
							{/each}
						</ul>
						{#if signals.hasMore}
							<Button
								variant="secondary"
								class="self-center"
								disabled={signals.loadingMore}
								onclick={() => signals.loadMore()}
							>
								{signals.loadingMore ? 'Loading…' : 'Load more'}
							</Button>
						{/if}
					</div>
				{/if}
			{:else if log.state.kind === 'loading'}
				{@render skeleton('Loading action log')}
			{:else if log.state.kind === 'error'}
				<div in:fade={{ duration: motionDuration('base') }}>
					<EmptyState
						title="Couldn't load the action log"
						description="Check that the API is running, then try again."
					>
						{#snippet action()}
							<Button variant="secondary" onclick={() => log.load()}>Try again</Button>
						{/snippet}
					</EmptyState>
				</div>
			{:else if log.actions.length === 0}
				<div in:fade={{ duration: motionDuration('base') }}>
					<EmptyState
						title="No agent actions yet"
						description="Every change an agent makes shows here, with undo."
					/>
				</div>
			{:else}
				<div class="flex flex-col gap-4">
					<ul class="flex flex-col gap-2" aria-label="Action log">
						{#each log.actions as action (action.id)}
							<li
								animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
								in:fade={{ duration: motionDuration('base'), easing: easings.enter }}
							>
								<ActionLogItem
									{action}
									busy={log.busy.has(action.id)}
									onundo={() => log.undo(action.id)}
								/>
							</li>
						{/each}
					</ul>
					{#if log.hasMore}
						<Button
							variant="secondary"
							class="self-center"
							disabled={log.loadingMore}
							onclick={() => log.loadMore()}
						>
							{log.loadingMore ? 'Loading…' : 'Load more'}
						</Button>
					{/if}
				</div>
			{/if}
		{/snippet}
	</Tabs>
</div>

{#if editing}
	{@const action = editing}
	<EditActionDialog
		bind:open={editOpen}
		input={action.input}
		onapprove={(input) => decided(queue.approve(action.id, input))}
	/>
{/if}

{#if rejecting}
	{@const action = rejecting}
	<RejectActionDialog
		bind:open={rejectOpen}
		busy={queue.busy.has(`action:${action.id}`)}
		onconfirm={() => decided(queue.reject(action.id))}
	/>
{/if}

{#if dismissing}
	{@const item = dismissing}
	<DismissReviewItemDialog
		bind:open={dismissOpen}
		busy={queue.busy.has(`review:${item.id}`)}
		ondismiss={(reason) => decided(queue.dismiss(item.id, reason))}
	/>
{/if}
