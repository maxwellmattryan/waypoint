<script lang="ts">
	import type { GoalStatus } from '#lib/api';
	import { Badge } from '#lib/components/ui';
	import { GOAL_STATUS_LABELS, GOAL_STATUS_TONES } from '#lib/goals/format.js';

	let { status }: { status: GoalStatus } = $props();
</script>

<Badge tone={GOAL_STATUS_TONES[status]} data-status={status}>{GOAL_STATUS_LABELS[status]}</Badge>
