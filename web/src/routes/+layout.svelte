<script lang="ts">
	import './layout.css';
	import { onMount } from 'svelte';
	import favicon from '#lib/assets/favicon.svg';
	import AppShell from '#lib/components/app/AppShell.svelte';
	import { Toaster } from '#lib/components/ui';
	import { Dashboard, setDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	// One controller for the app: the shell's connection badge and the dashboard page share it.
	const dashboard = setDashboard(new Dashboard());

	onMount(() => {
		dashboard.start();
		return () => dashboard.stop();
	});
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
</svelte:head>

<AppShell connection={dashboard.connection}>
	{@render children()}
</AppShell>
<Toaster />
