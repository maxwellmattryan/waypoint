<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { fade } from 'svelte/transition';
	import { createApiClient } from '#lib/api';
	import RunDetailView from '#lib/components/runs/RunDetailView.svelte';
	import { Button, EmptyState, Link, Skeleton } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { motionDuration } from '#lib/motion.js';
	import { RunDetailStore } from '#lib/runs/runs.svelte.js';

	const client = createApiClient();
	const dashboard = getDashboard();
	const listHref = resolve('/runs');

	/** The route's id as a positive integer, or `null` for anything else (no request is made). */
	const id = $derived.by(() => {
		const raw = page.params.id ?? '';
		const parsed = /^[1-9]\d*$/.test(raw) ? Number(raw) : null;
		return parsed !== null && Number.isSafeInteger(parsed) ? parsed : null;
	});

	let store = $state<RunDetailStore | null>(null);

	$effect(() => {
		if (id === null) {
			store = null;
			return;
		}
		const next = new RunDetailStore(id, { client });
		store = next;
		void next.load();
		return dashboard.subscribe((event) => next.handle(event));
	});

	const view = $derived(store?.state ?? { kind: 'not-found' as const });
</script>

<svelte:head>
	<title>{view.kind === 'loaded' ? `${view.run.workflow} #${view.run.id}` : 'Run'} · Waypoint</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<Link href={listHref} class="self-start text-sm">Back to runs</Link>

	{#if view.kind === 'loading'}
		<div class="flex flex-col gap-4" aria-busy="true" aria-label="Loading run">
			<Skeleton class="h-8 w-64" />
			<div class="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
				{#each [0, 1, 2, 3, 4] as tile (tile)}
					<Skeleton class="h-20" />
				{/each}
			</div>
			<Skeleton class="h-40" />
		</div>
	{:else if view.kind === 'not-found'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState title="Run not found" description="The link may be wrong.">
				{#snippet action()}
					<Link href={listHref}>All runs</Link>
				{/snippet}
			</EmptyState>
		</div>
	{:else if view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load this run"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => store?.load()}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		<div in:fade={{ duration: motionDuration('base') }}>
			<RunDetailView run={view.run} />
		</div>
	{/if}
</div>
