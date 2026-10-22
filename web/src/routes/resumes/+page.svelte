<script lang="ts">
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { createApiClient, type ResumeVersion } from '#lib/api';
	import { listResumes, uploadResume } from '#lib/resumes/api.js';
	import ResumeListItem from '#lib/components/resumes/ResumeListItem.svelte';
	import ResumeUploadDialog from '#lib/components/resumes/ResumeUploadDialog.svelte';
	import { Button, EmptyState, Skeleton } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';

	type View =
		| { kind: 'loading' }
		| { kind: 'error' }
		| { kind: 'loaded'; versions: ResumeVersion[] };

	const client = createApiClient();
	let view = $state<View>({ kind: 'loading' });
	let uploading = $state(false);

	async function load() {
		view = { kind: 'loading' };
		const result = await listResumes(client);
		view = result.kind === 'ok' ? { kind: 'loaded', versions: result.data } : { kind: 'error' };
	}

	function uploaded(version: ResumeVersion) {
		// The newest upload sorts first.
		if (view.kind === 'loaded') view.versions = [version, ...view.versions];
		else void load();
	}

	onMount(() => {
		void load();
	});
</script>

<svelte:head>
	<title>Resumes · Waypoint</title>
</svelte:head>

{#snippet uploadButton()}
	<Button onclick={() => (uploading = true)}>Upload resume</Button>
{/snippet}

<div class="flex flex-col gap-8">
	<div class="flex flex-wrap items-center justify-between gap-4">
		<h1 class="text-2xl font-semibold text-fg">Resumes</h1>
		{@render uploadButton()}
	</div>

	{#if view.kind === 'loading'}
		<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading resumes">
			{#each [0, 1, 2] as row (row)}
				<Skeleton class="h-20" />
			{/each}
		</div>
	{:else if view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load resumes"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={load}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if view.versions.length === 0}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="No resumes yet"
				description="Upload each PDF you send out, so you can see what every company received."
			>
				{#snippet action()}
					{@render uploadButton()}
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		<ul class="flex flex-col gap-2" aria-label="Resumes">
			{#each view.versions as version (version.id)}
				<li
					animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
					transition:fade={{ duration: motionDuration('fast') }}
				>
					<ResumeListItem {version} />
				</li>
			{/each}
		</ul>
	{/if}
</div>

<ResumeUploadDialog
	bind:open={uploading}
	save={(file, name) => uploadResume(client, file, name)}
	onsaved={uploaded}
/>
