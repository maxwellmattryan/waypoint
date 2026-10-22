<script lang="ts">
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { createApiClient, type Application } from '#lib/api';
	import { createApplication, listApplications } from '#lib/applications/api.js';
	import ApplicationFormDialog from '#lib/components/applications/ApplicationFormDialog.svelte';
	import ApplicationListItem from '#lib/components/applications/ApplicationListItem.svelte';
	import { Button, EmptyState, Skeleton } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';

	type View =
		| { kind: 'loading' }
		| { kind: 'error' }
		| { kind: 'loaded'; applications: Application[] };

	const client = createApiClient();
	let view = $state<View>({ kind: 'loading' });
	let adding = $state(false);

	async function load() {
		view = { kind: 'loading' };
		const result = await listApplications(client);
		view = result.kind === 'ok' ? { kind: 'loaded', applications: result.data } : { kind: 'error' };
	}

	function added(application: Application) {
		// The newest update sorts first, so a new application goes on top.
		if (view.kind === 'loaded') view.applications = [application, ...view.applications];
		else void load();
	}

	onMount(() => {
		void load();
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

	{#if view.kind === 'loading'}
		<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading applications">
			{#each [0, 1, 2] as row (row)}
				<Skeleton class="h-20" />
			{/each}
		</div>
	{:else if view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load applications"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={load}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if view.applications.length === 0}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState title="No applications yet" description="Add one to start tracking.">
				{#snippet action()}
					{@render addButton()}
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		<ul class="flex flex-col gap-2" aria-label="Applications">
			{#each view.applications as application (application.id)}
				<li
					animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
					transition:fade={{ duration: motionDuration('fast') }}
				>
					<ApplicationListItem {application} />
				</li>
			{/each}
		</ul>
	{/if}
</div>

<ApplicationFormDialog
	bind:open={adding}
	mode="create"
	save={(values) => createApplication(client, values)}
	onsaved={added}
/>
