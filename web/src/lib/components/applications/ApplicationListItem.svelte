<script lang="ts">
	import { resolve } from '$app/paths';
	import type { Application } from '#lib/api';
	import { formatDate } from '#lib/applications/format.js';
	import { Card } from '#lib/components/ui';
	import StatusBadge from './StatusBadge.svelte';

	let { application }: { application: Application } = $props();

	const href = $derived(resolve('/applications/[id]', { id: String(application.id) }));
</script>

<Card {href}>
	<div class="flex items-start justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<p class="truncate text-sm text-fg">
				<span class="font-semibold">{application.company}</span>
				<span class="text-fg-subtle" aria-hidden="true">·</span>
				<span class="text-fg-muted">{application.title}</span>
			</p>
			<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted">
				{#if application.location}
					<span>{application.location}</span>
				{/if}
				<span>Updated {formatDate(application.updated_at)}</span>
			</p>
		</div>
		<StatusBadge status={application.status} />
	</div>
</Card>
