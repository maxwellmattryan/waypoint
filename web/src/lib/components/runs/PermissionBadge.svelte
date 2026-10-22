<script lang="ts">
	import type { Permission } from '#lib/api';
	import { permissionSpec } from '#lib/runs/statuses.js';
	import { Badge } from '#lib/components/ui';

	let { permission }: { permission: Permission | null } = $props();

	const spec = $derived(permissionSpec(permission));
</script>

<Badge
	tone={spec.tone}
	data-permission={permission ?? 'unknown'}
	aria-label="Permission: {spec.label}"
>
	{spec.label}
</Badge>
