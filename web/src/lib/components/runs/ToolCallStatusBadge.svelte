<script lang="ts">
	import type { ToolCallStatus } from '#lib/api';
	import { toolCallStatusBadge } from '#lib/runs/statuses.js';
	import { Badge } from '#lib/components/ui';

	let { status }: { status: ToolCallStatus } = $props();

	const spec = $derived(toolCallStatusBadge[status]);
</script>

<Badge tone={spec.tone} data-status={status}>{spec.label}</Badge>
