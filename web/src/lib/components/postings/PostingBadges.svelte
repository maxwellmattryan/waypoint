<script lang="ts">
	import type { JobPosting } from '#lib/api';
	import { Badge } from '#lib/components/ui';
	import { SOURCE_LABELS } from '#lib/postings/api.js';

	let { posting }: { posting: JobPosting } = $props();
</script>

<div class="flex flex-wrap items-center gap-2">
	<Badge>{SOURCE_LABELS[posting.source]}</Badge>
	{#if posting.remote}
		<Badge tone="info">Remote</Badge>
	{/if}
	{#if posting.removed_at}
		<Badge tone="warning">Removed</Badge>
	{/if}
</div>
