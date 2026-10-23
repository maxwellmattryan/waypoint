<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { fade } from 'svelte/transition';
	import { createApiClient } from '#lib/api';
	import PostingDetailView from '#lib/components/postings/PostingDetailView.svelte';
	import PostingFitPanel from '#lib/components/postings/PostingFitPanel.svelte';
	import { Button, EmptyState, Link, Skeleton } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import PostingPacketsPanel from '#lib/components/packets/PostingPacketsPanel.svelte';
	import { motionDuration } from '#lib/motion.js';
	import { PostingPacketsStore } from '#lib/packets/packets.svelte.js';
	import { PostingDetailStore, PostingFitStore } from '#lib/postings/postings.svelte.js';

	const client = createApiClient();
	const dashboard = getDashboard();
	const listHref = resolve('/postings');

	/** The route's id as a positive integer, or `null` for anything else (no request is made). */
	const id = $derived.by(() => {
		const raw = page.params.id ?? '';
		const parsed = /^[1-9]\d*$/.test(raw) ? Number(raw) : null;
		return parsed !== null && Number.isSafeInteger(parsed) ? parsed : null;
	});

	let store = $state<PostingDetailStore | null>(null);
	let fitStore = $state<PostingFitStore | null>(null);
	let packetsStore = $state<PostingPacketsStore | null>(null);

	$effect(() => {
		if (id === null) {
			store = null;
			fitStore = null;
			packetsStore = null;
			return;
		}
		const next = new PostingDetailStore(id, { client });
		store = next;
		void next.load();
		const nextFit = new PostingFitStore(id, { client });
		fitStore = nextFit;
		void nextFit.load();
		const nextPackets = new PostingPacketsStore(id, { client });
		packetsStore = nextPackets;
		void nextPackets.load();
		const unsubscribe = dashboard.subscribe((event) => {
			nextFit.handle(event);
			nextPackets.handle(event);
		});
		return () => {
			unsubscribe();
			nextFit.destroy();
			nextPackets.destroy();
		};
	});

	const view = $derived(store?.state ?? { kind: 'not-found' as const });
</script>

<svelte:head>
	<title>{view.kind === 'loaded' ? view.posting.title : 'Posting'} · Waypoint</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<Link href={listHref} class="self-start text-sm">Back to postings</Link>

	{#if view.kind === 'loading'}
		<div class="flex flex-col gap-4" aria-busy="true" aria-label="Loading posting">
			<Skeleton class="h-8 w-64" />
			<Skeleton class="h-48" />
			<Skeleton class="h-40" />
		</div>
	{:else if view.kind === 'not-found'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState title="Posting not found" description="The link may be wrong.">
				{#snippet action()}
					<Link href={listHref}>All postings</Link>
				{/snippet}
			</EmptyState>
		</div>
	{:else if view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load this posting"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => store?.load()}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		<div in:fade={{ duration: motionDuration('base') }}>
			<PostingDetailView posting={view.posting}>
				{#snippet fit()}
					{#if fitStore}
						<PostingFitPanel
							state={fitStore.state}
							starting={fitStore.starting}
							onscore={() => fitStore?.score()}
							onreload={() => fitStore?.load()}
						/>
					{/if}
				{/snippet}
				{#snippet packets()}
					{#if packetsStore}
						<PostingPacketsPanel
							state={packetsStore.state}
							starting={packetsStore.starting}
							onprepare={() => packetsStore?.prepare()}
							onreload={() => packetsStore?.load()}
						/>
					{/if}
				{/snippet}
			</PostingDetailView>
		</div>
	{/if}
</div>
