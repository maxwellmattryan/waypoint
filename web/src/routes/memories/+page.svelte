<script lang="ts">
	import { onMount } from 'svelte';
	import { fade } from 'svelte/transition';
	import { createApiClient, type Memory, type MemoryInput } from '#lib/api';
	import MemoryFilters from '#lib/components/memories/MemoryFilters.svelte';
	import MemoryFormDialog from '#lib/components/memories/MemoryFormDialog.svelte';
	import MemorySection from '#lib/components/memories/MemorySection.svelte';
	import { Button, ConfirmDialog, EmptyState, Skeleton } from '#lib/components/ui';
	import { createMemory, updateMemory } from '#lib/memories/api.js';
	import { MemoryList } from '#lib/memories/memories.svelte.js';
	import { SECTIONS } from '#lib/memories/topics.js';
	import { motionDuration } from '#lib/motion.js';

	const client = createApiClient();
	const list = new MemoryList({ client });

	let formOpen = $state(false);
	/** The memory the dialog edits; `null` adds one. */
	let editing = $state<Memory | null>(null);
	let deleteOpen = $state(false);
	let deleting = $state<Memory | null>(null);

	const sections = $derived(
		SECTIONS.map((section) => ({
			...section,
			memories: list.memories.filter((memory) => memory.kind === section.kind)
		})).filter((section) => section.memories.length > 0)
	);

	function add() {
		editing = null;
		formOpen = true;
	}

	function edit(memory: Memory) {
		editing = memory;
		formOpen = true;
	}

	function askDelete(memory: Memory) {
		deleting = memory;
		deleteOpen = true;
	}

	function save(input: MemoryInput) {
		return editing ? updateMemory(client, editing.id, input) : createMemory(client, input);
	}

	async function confirmDelete(): Promise<boolean> {
		return deleting ? list.delete(deleting.id) : true;
	}

	onMount(() => {
		void list.load();
		return () => list.destroy();
	});
</script>

<svelte:head>
	<title>Memory · Waypoint</title>
</svelte:head>

{#snippet addButton()}
	<Button onclick={add}>Add memory</Button>
{/snippet}

<div class="flex flex-col gap-8">
	<div class="flex flex-wrap items-center justify-between gap-4">
		<h1 class="text-2xl font-semibold text-fg">What Waypoint knows about me</h1>
		{@render addButton()}
	</div>

	<MemoryFilters
		q={list.filters.q}
		status={list.filters.status}
		tag={list.filters.tag}
		onquery={(q) => list.setQuery(q)}
		onstatus={(status) => list.setStatus(status)}
		oncleartag={() => list.setTag('')}
	/>

	{#if list.view.kind === 'loading'}
		<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading memories">
			{#each [0, 1, 2] as row (row)}
				<Skeleton class="h-24" />
			{/each}
		</div>
	{:else if list.view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load memories"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => list.load()}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if sections.length === 0 && list.filtered}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="No memories match"
				description="Try other words, or clear the filters to see everything."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => list.clearFilters()}>Clear filters</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if sections.length === 0}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Waypoint doesn't know anything about you yet"
				description="Add facts, preferences and answers you give often. Waypoint uses them to prepare applications."
			>
				{#snippet action()}
					{@render addButton()}
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		{#each sections as section (section.kind)}
			<div transition:fade={{ duration: motionDuration('fast') }}>
				<MemorySection
					heading={section.heading}
					memories={section.memories}
					onconfirm={(memory) => list.confirm(memory.id)}
					onedit={edit}
					ondelete={askDelete}
					ontag={(tag) => list.setTag(tag)}
				/>
			</div>
		{/each}
	{/if}
</div>

<MemoryFormDialog
	bind:open={formOpen}
	memory={editing}
	{save}
	onsaved={(memory, wasNew) => list.saved(memory, wasNew)}
/>

<ConfirmDialog
	bind:open={deleteOpen}
	title="Delete memory"
	description="Waypoint will forget this. This can't be undone."
	onconfirm={confirmDelete}
/>
