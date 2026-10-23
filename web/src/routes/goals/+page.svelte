<script lang="ts">
	import { onMount } from 'svelte';
	import { createApiClient } from '#lib/api';
	import GoalsView from '#lib/components/goals/GoalsView.svelte';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { GoalsStore } from '#lib/goals/goals.svelte.js';

	const store = new GoalsStore({ client: createApiClient() });
	const dashboard = getDashboard();

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
	<title>Goals · Waypoint</title>
</svelte:head>

<GoalsView
	view={store.view}
	goals={store.goals}
	creating={store.creating}
	error={store.error}
	oncreate={(text) => store.create(text)}
	onreload={() => store.load()}
/>
