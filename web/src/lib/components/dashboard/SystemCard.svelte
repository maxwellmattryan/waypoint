<script lang="ts" module>
	import type { BadgeTone } from '#lib/components/ui';
	import type { HealthState } from '#lib/dashboard/dashboard.svelte.js';

	export interface SystemCardProps {
		health: HealthState;
	}

	type Row = { tone: BadgeTone; value: string };

	const rows: Record<Exclude<HealthState['kind'], 'loading'>, { api: Row; db: Row }> = {
		ok: { api: { tone: 'success', value: 'OK' }, db: { tone: 'success', value: 'OK' } },
		degraded: {
			api: { tone: 'success', value: 'OK' },
			db: { tone: 'danger', value: 'Unavailable' }
		},
		unreachable: {
			api: { tone: 'danger', value: 'Unreachable' },
			db: { tone: 'neutral', value: 'Unknown' }
		}
	};
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { Skeleton } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';
	import DashboardCard from './DashboardCard.svelte';
	import StatusRow from './StatusRow.svelte';

	let { health }: SystemCardProps = $props();
</script>

<DashboardCard title="System">
	{#key health.kind}
		<div in:fade={{ duration: motionDuration('base') }}>
			{#if health.kind === 'loading'}
				<div class="flex flex-col gap-4" data-testid="system-loading">
					<Skeleton shape="text" class="w-full" />
					<Skeleton shape="text" class="w-full" />
				</div>
			{:else}
				<dl class="flex flex-col gap-4">
					<StatusRow label="API" {...rows[health.kind].api} />
					<StatusRow label="Database" {...rows[health.kind].db} />
				</dl>
			{/if}
		</div>
	{/key}
</DashboardCard>
