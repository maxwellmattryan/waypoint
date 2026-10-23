<script lang="ts">
	import { onMount } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { createApiClient } from '#lib/api';
	import PostingFilters from '#lib/components/postings/PostingFilters.svelte';
	import PostingListItem from '#lib/components/postings/PostingListItem.svelte';
	import SourceCompanyDialog, {
		type QueuedBoard
	} from '#lib/components/postings/SourceCompanyDialog.svelte';
	import { Button, EmptyState, Skeleton, toast } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { easings, motionDuration } from '#lib/motion.js';
	import { PostingsStore } from '#lib/postings/postings.svelte.js';

	const client = createApiClient();
	const store = new PostingsStore({ client });
	const dashboard = getDashboard();

	let sourceOpen = $state(false);

	function queued({ jobId, slug, reused }: QueuedBoard) {
		store.track(jobId, slug);
		toast.success(reused ? `Already fetching ${slug}` : `Fetching ${slug}…`);
	}

	onMount(() => {
		void store.load();
		const unsubscribe = dashboard.subscribe((event) => store.handle(event));
		return () => {
			unsubscribe();
			store.destroy();
		};
	});
</script>

<svelte:head>
	<title>Postings · Waypoint</title>
</svelte:head>

{#snippet sourceButton()}
	<Button onclick={() => (sourceOpen = true)}>Source company</Button>
{/snippet}

<div class="flex flex-col gap-8">
	<div class="flex flex-wrap items-center justify-between gap-4">
		<div class="flex flex-col gap-1">
			<h1 class="text-2xl font-semibold text-fg">Postings</h1>
			<p class="text-sm text-fg-muted">Open roles from the company boards you source.</p>
		</div>
		{@render sourceButton()}
	</div>

	<PostingFilters
		filters={store.filters}
		oncompany={(company) => store.setCompany(company)}
		onlocation={(location) => store.setLocation(location)}
		onsource={(source) => store.setSource(source)}
		onremote={(remote) => store.setRemote(remote)}
		onincluderemoved={(includeRemoved) => store.setIncludeRemoved(includeRemoved)}
	/>

	{#if store.view.kind === 'loading'}
		<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading postings">
			{#each [0, 1, 2] as row (row)}
				<Skeleton class="h-24" />
			{/each}
		</div>
	{:else if store.view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load postings"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => store.load()}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if store.postings.length === 0 && store.filtered}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState title="No postings match these filters">
				{#snippet action()}
					<Button variant="secondary" onclick={() => store.clearFilters()}>Clear filters</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if store.postings.length === 0}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="No postings yet"
				description="Source a company's Greenhouse, Lever or Ashby board to see its open roles here."
			>
				{#snippet action()}
					{@render sourceButton()}
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		<div class="flex flex-col gap-4">
			<ul class="flex flex-col gap-2" aria-label="Postings">
				{#each store.postings as posting (posting.id)}
					<li
						animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
						in:fade={{ duration: motionDuration('base') }}
					>
						<PostingListItem {posting} />
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

<SourceCompanyDialog bind:open={sourceOpen} {client} onqueued={queued} />
