<script lang="ts">
	import type { StepStatus } from '#lib/api';
	import { stepStatusBadge } from '#lib/runs/statuses.js';
	import { Badge } from '#lib/components/ui';

	let { status }: { status: StepStatus } = $props();

	const spec = $derived(stepStatusBadge[status]);
</script>

<Badge tone={spec.tone} data-status={status}>{spec.label}</Badge>
