<script lang="ts">
	import { resolve } from '$app/paths';
	import type { ResumeVersion } from '#lib/api';
	import { formatDate } from '#lib/applications/format.js';
	import { formatBytes, usedBy } from '#lib/resumes/format.js';
	import { Badge, Card } from '#lib/components/ui';

	let { version }: { version: ResumeVersion } = $props();

	const href = $derived(resolve('/resumes/[id]', { id: String(version.id) }));
</script>

<Card {href}>
	<div class="flex items-start justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<p class="truncate text-sm text-fg">
				<span class="font-semibold">{version.name}</span>
				<span class="text-fg-subtle" aria-hidden="true">·</span>
				<span class="text-fg-muted">{version.filename}</span>
			</p>
			<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted">
				<span>{formatBytes(version.size_bytes)}</span>
				<span>Uploaded {formatDate(version.created_at)}</span>
				{#if version.application_count > 0}
					<span>{usedBy(version.application_count)}</span>
				{/if}
			</p>
		</div>
		{#if !version.has_text}
			<Badge tone="warning">No text</Badge>
		{/if}
	</div>
</Card>
