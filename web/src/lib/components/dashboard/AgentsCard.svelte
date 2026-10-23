<script lang="ts" module>
	import type { KillSwitchState } from '#lib/dashboard/dashboard.svelte.js';

	export interface AgentsCardProps {
		killSwitch: KillSwitchState;
		/** Whether a change is in flight. */
		saving: boolean;
		/** Asks to pause (`true`) or resume (`false`) every agent. */
		ontoggle: (paused: boolean) => void;
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { Badge, Button, Skeleton } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';
	import DashboardCard from './DashboardCard.svelte';

	let { killSwitch, saving, ontoggle }: AgentsCardProps = $props();

	/** One key per visible state. */
	const view = $derived(
		killSwitch.kind !== 'loaded' ? killSwitch.kind : killSwitch.state.paused ? 'paused' : 'running'
	);
	const label = $derived(
		view === 'paused'
			? saving
				? 'Resuming…'
				: 'Resume agents'
			: saving
				? 'Pausing…'
				: 'Pause agents'
	);
</script>

<DashboardCard title="Agents">
	{#snippet status()}
		{#if view === 'running'}
			<Badge tone="success" data-testid="agents-status">Running</Badge>
		{:else if view === 'paused'}
			<Badge tone="warning" data-testid="agents-status">Paused</Badge>
		{/if}
	{/snippet}

	{#key view}
		<div class="flex flex-col gap-4" in:fade={{ duration: motionDuration('base') }}>
			{#if view === 'loading'}
				<div class="flex flex-col gap-4" data-testid="agents-loading">
					<Skeleton shape="text" class="w-full" />
					<Skeleton shape="text" class="w-2/3" />
				</div>
			{:else if view === 'error'}
				<p class="text-sm text-fg-muted">Couldn't load the agent status.</p>
			{:else if view === 'paused'}
				<p class="text-sm text-fg-muted">
					Agents are paused. Queued and new runs wait; running runs stop after their current step.
				</p>
				<div>
					<Button variant="primary" disabled={saving} onclick={() => ontoggle(false)}>{label}</Button>
				</div>
			{:else}
				<p class="text-sm text-fg-muted">Agents can start and continue runs.</p>
				<div>
					<Button variant="danger" disabled={saving} onclick={() => ontoggle(true)}>{label}</Button>
				</div>
			{/if}
		</div>
	{/key}
</DashboardCard>
