<script lang="ts">
	import ClaudeCard from '#lib/components/dashboard/ClaudeCard.svelte';
	import GoogleCard from '#lib/components/dashboard/GoogleCard.svelte';
	import SystemCard from '#lib/components/dashboard/SystemCard.svelte';
	import { EmptyState } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';

	const dashboard = getDashboard();
</script>

<svelte:head>
	<title>Waypoint</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<h1 class="text-2xl font-semibold text-fg">Dashboard</h1>

	<div class="grid gap-4 lg:grid-cols-3">
		<SystemCard health={dashboard.health} />
		<GoogleCard google={dashboard.google} />
		<ClaudeCard
			test={dashboard.test}
			testing={dashboard.testing}
			queuePausedUntil={dashboard.queuePausedUntil}
			ontest={() => dashboard.runTest()}
		/>
	</div>

	<EmptyState
		title="No applications yet"
		description="Application tracking arrives in the next milestone."
	/>
</div>
