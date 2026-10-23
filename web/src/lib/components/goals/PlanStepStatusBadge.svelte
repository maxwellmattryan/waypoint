<script lang="ts">
	import type { PlanStepStatus } from '#lib/api';
	import { Badge } from '#lib/components/ui';
	import { STEP_STATUS_LABELS, STEP_STATUS_TONES } from '#lib/goals/format.js';

	let { status }: { status: PlanStepStatus } = $props();
</script>

<Badge tone={STEP_STATUS_TONES[status]} data-status={status}>{STEP_STATUS_LABELS[status]}</Badge>
