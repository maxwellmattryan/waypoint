<script lang="ts">
	import type { ApplicationStatus } from '#lib/api';
	import { statusLabel, statusTone } from '#lib/applications/statuses.js';
	import { Badge } from '#lib/components/ui';

	let { status }: { status: ApplicationStatus } = $props();
</script>

<Badge tone={statusTone[status]} data-status={status}>{statusLabel[status]}</Badge>
