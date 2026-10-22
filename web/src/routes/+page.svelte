<script lang="ts">
	import AgentsCard from '#lib/components/dashboard/AgentsCard.svelte';
	import ClaudeCard from '#lib/components/dashboard/ClaudeCard.svelte';
	import DataCard from '#lib/components/dashboard/DataCard.svelte';
	import GoogleCard from '#lib/components/dashboard/GoogleCard.svelte';
	import SystemCard from '#lib/components/dashboard/SystemCard.svelte';
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
		<AgentsCard
			killSwitch={dashboard.killSwitch}
			saving={dashboard.savingKillSwitch}
			ontoggle={(paused) => dashboard.setPaused(paused)}
		/>
		<DataCard ondelete={(confirmation) => dashboard.deleteAllData(confirmation)} />
	</div>
</div>
