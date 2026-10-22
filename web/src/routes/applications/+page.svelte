<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { createApiClient, type Application } from '#lib/api';
	import { createApplication } from '#lib/applications/api.js';
	import { ApplicationsStore } from '#lib/applications/applications.svelte.js';
	import ApplicationBoard from '#lib/components/applications/ApplicationBoard.svelte';
	import ApplicationFormDialog from '#lib/components/applications/ApplicationFormDialog.svelte';
	import ApplicationListItem from '#lib/components/applications/ApplicationListItem.svelte';
	import { Button, EmptyState, Skeleton, Tabs } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { easings, motionDuration } from '#lib/motion.js';

	type ViewName = 'list' | 'board';

	const client = createApiClient();
	const store = new ApplicationsStore({ client });
	const dashboard = getDashboard();
	const views = [
		{ value: 'list', label: 'List' },
		{ value: 'board', label: 'Board' }
	];

	let adding = $state(false);

	/** The tab, mirrored in `?view=board`; any other value is the list. */
	const view = $derived<ViewName>(page.url.searchParams.get('view') === 'board' ? 'board' : 'list');

	function showView(next: string) {
		if (next === view) return;
		// Replace the history entry and leave scroll and focus where they are.
		const options = { replace: true, reset: false };
		void goto(
			next === 'board' ? resolve('/applications?view=board') : resolve('/applications'),
			options
		);
	}

	function added(application: Application) {
		if (store.state.kind === 'loaded') store.upsert(application);
		else void store.load();
	}

	onMount(() => {
		void store.load();
		return dashboard.subscribe((event) => store.apply(event));
	});
</script>

<svelte:head>
	<title>Applications · Waypoint</title>
</svelte:head>

{#snippet addButton()}
	<Button onclick={() => (adding = true)}>Add application</Button>
{/snippet}

<div class="flex flex-col gap-8">
	<div class="flex flex-wrap items-center justify-between gap-4">
		<h1 class="text-2xl font-semibold text-fg">Applications</h1>
		{@render addButton()}
	</div>

	{#if store.state.kind === 'loading'}
		{#if view === 'board'}
			<div
				class="flex gap-4 overflow-x-auto pb-4 lg:grid lg:grid-cols-5 lg:overflow-visible"
				aria-busy="true"
				aria-label="Loading applications"
			>
				{#each [0, 1, 2, 3, 4] as column (column)}
					<div class="flex w-64 shrink-0 flex-col gap-2 lg:w-auto">
						<Skeleton shape="text" class="h-6 w-24" />
						<div class="flex flex-col gap-2 rounded-lg bg-surface-raised p-2">
							<Skeleton class="h-20 bg-surface" />
							<Skeleton class="h-20 bg-surface" />
						</div>
					</div>
				{/each}
			</div>
		{:else}
			<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading applications">
				{#each [0, 1, 2] as row (row)}
					<Skeleton class="h-20" />
				{/each}
			</div>
		{/if}
	{:else if store.state.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load applications"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => store.load()}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if store.applications.length === 0}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState title="No applications yet" description="Add one to start tracking.">
				{#snippet action()}
					{@render addButton()}
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		<Tabs label="View" items={views} bind:value={() => view, showView}>
			{#snippet children(active)}
				{#if active === 'board'}
					<ApplicationBoard
						columns={store.columns}
						pending={store.pending}
						onmove={(id, status) => store.move(id, status)}
					/>
				{:else}
					<ul class="flex flex-col gap-2" aria-label="Applications">
						{#each store.applications as application (application.id)}
							<li
								animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
								transition:fade={{ duration: motionDuration('fast') }}
							>
								<ApplicationListItem {application} />
							</li>
						{/each}
					</ul>
				{/if}
			{/snippet}
		</Tabs>
	{/if}
</div>

<ApplicationFormDialog
	bind:open={adding}
	mode="create"
	save={(values) => createApplication(client, values)}
	onsaved={added}
/>
