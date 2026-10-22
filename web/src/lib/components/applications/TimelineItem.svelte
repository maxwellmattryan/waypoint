<script lang="ts" module>
	import type { ApplicationEvent } from '#lib/api';

	export interface TimelineItemProps {
		event: ApplicationEvent;
	}
</script>

<script lang="ts">
	import { formatDateTime } from '#lib/applications/format.js';
	import { describeEvent } from '#lib/applications/timeline.js';
	import { Badge } from '#lib/components/ui';

	let { event }: TimelineItemProps = $props();
</script>

<div class="flex flex-col gap-1">
	{#if event.kind === 'rejection_feedback'}
		<!-- The badge is the description, so it isn't read twice. -->
		<Badge tone="danger" class="self-start">{describeEvent(event)}</Badge>
	{:else}
		<p class="text-sm font-medium break-words text-fg">{describeEvent(event)}</p>
	{/if}
	<time datetime={event.created_at} class="text-xs text-fg-muted">
		{formatDateTime(event.created_at)}
	</time>
	{#if event.body && (event.kind === 'note' || event.kind === 'rejection_feedback')}
		<p class="text-sm break-words whitespace-pre-wrap text-fg">{event.body}</p>
	{/if}
</div>
