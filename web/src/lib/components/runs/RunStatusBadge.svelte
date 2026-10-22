<script lang="ts">
	import type { RunStatus } from '#lib/api';
	import { runStatusBadge } from '#lib/runs/statuses.js';
	import { Badge } from '#lib/components/ui';

	let { status }: { status: RunStatus } = $props();

	const spec = $derived(runStatusBadge[status]);
</script>

<Badge tone={spec.tone} data-status={status}>{spec.label}</Badge>
