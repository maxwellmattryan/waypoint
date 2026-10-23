<script lang="ts">
	import { page } from '$app/state';
	import { fade } from 'svelte/transition';
	import { createApiClient } from '#lib/api';
	import PacketDetailView from '#lib/components/packets/PacketDetailView.svelte';
	import { Button, EmptyState, Skeleton } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { motionDuration } from '#lib/motion.js';
	import { PacketStore } from '#lib/packets/packets.svelte.js';

	const client = createApiClient();
	const dashboard = getDashboard();

	/** The route's id as a positive integer, or `null` for anything else (no request is made). */
	const id = $derived.by(() => {
		const raw = page.params.id ?? '';
		const parsed = /^[1-9]\d*$/.test(raw) ? Number(raw) : null;
		return parsed !== null && Number.isSafeInteger(parsed) ? parsed : null;
	});

	let store = $state<PacketStore | null>(null);

	$effect(() => {
		if (id === null) {
			store = null;
			return;
		}
		const next = new PacketStore(id, { client });
		store = next;
		void next.load();
		const unsubscribe = dashboard.subscribe((event) => next.handle(event));
		return () => {
			unsubscribe();
			next.destroy();
		};
	});

	const view = $derived(store?.state ?? { kind: 'not-found' as const });
</script>

<svelte:head>
	<title>
		{view.kind === 'loaded' ? `Packet · ${view.packet.company}` : 'Packet'} · Waypoint
	</title>
</svelte:head>

{#if view.kind === 'loading'}
	<div class="flex flex-col gap-4" aria-busy="true" aria-label="Loading packet">
		<Skeleton class="h-8 w-64" />
		<Skeleton class="h-24" />
		<Skeleton class="h-48" />
	</div>
{:else if view.kind === 'not-found'}
	<div in:fade={{ duration: motionDuration('base') }}>
		<EmptyState title="Packet not found" description="The link may be wrong." />
	</div>
{:else if view.kind === 'error'}
	<div in:fade={{ duration: motionDuration('base') }}>
		<EmptyState
			title="Couldn't load this packet"
			description="Check that the API is running, then try again."
		>
			{#snippet action()}
				<Button variant="secondary" onclick={() => store?.load()}>Try again</Button>
			{/snippet}
		</EmptyState>
	</div>
{:else}
	<div in:fade={{ duration: motionDuration('base') }}>
		<PacketDetailView packet={view.packet} />
	</div>
{/if}
