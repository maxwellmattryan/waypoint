<script lang="ts">
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { createApiClient } from '#lib/api';
	import RunListItem from '#lib/components/runs/RunListItem.svelte';
	import { Button, EmptyState, Skeleton } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { easings, motionDuration } from '#lib/motion.js';
	import { RunsStore } from '#lib/runs/runs.svelte.js';

	const store = new RunsStore({ client: createApiClient() });
	const dashboard = getDashboard();

	onMount(() => {
		void store.load();
		return dashboard.subscribe((event) => store.handle(event));
	});
</script>

<svelte:head>
	<title>Runs · Waypoint</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<div class="flex flex-col gap-1">
		<h1 class="text-2xl font-semibold text-fg">Runs</h1>
		<p class="text-sm text-fg-muted">Every agent run, its steps, tool calls and what it cost.</p>
	</div>

	{#if store.state.kind === 'loading'}
		<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading runs">
			{#each [0, 1, 2] as row (row)}
				<Skeleton class="h-20" />
			{/each}
		</div>
	{:else if store.state.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load runs"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => store.load()}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if store.runs.length === 0}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="No runs yet"
				description="Runs show here as soon as an agent workflow starts."
			/>
		</div>
	{:else}
		<div class="flex flex-col gap-4">
			<ul class="flex flex-col gap-2" aria-label="Runs">
				{#each store.runs as run (run.id)}
					<li
						animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
						in:fade={{ duration: motionDuration('base') }}
					>
						<RunListItem {run} />
					</li>
				{/each}
			</ul>
			{#if store.hasMore}
				<Button
					variant="secondary"
					class="self-center"
					disabled={store.loadingMore}
					onclick={() => store.loadMore()}
				>
					{store.loadingMore ? 'Loading…' : 'Load more'}
				</Button>
			{/if}
		</div>
	{/if}
</div>
