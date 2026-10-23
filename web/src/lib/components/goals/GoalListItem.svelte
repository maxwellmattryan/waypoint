<script lang="ts">
	import type { Goal } from '#lib/api';
	import { Card } from '#lib/components/ui';
	import { goalHref } from '#lib/goals/goals.svelte.js';
	import { timeAgo } from '#lib/goals/format.js';
	import GoalStatusBadge from './GoalStatusBadge.svelte';

	/** One goal in the list, linking to its page. */
	let { goal }: { goal: Goal } = $props();
</script>

<Card href={goalHref(goal.id)}>
	<div class="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
		<p class="min-w-0 text-sm font-semibold break-words text-fg">{goal.text}</p>
		<div class="flex shrink-0 items-center gap-2">
			<span class="text-xs text-fg-muted">Updated {timeAgo(goal.updated_at)}</span>
			<GoalStatusBadge status={goal.status} />
		</div>
	</div>
</Card>
