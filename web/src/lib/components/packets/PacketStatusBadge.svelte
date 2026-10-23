<script lang="ts" module>
	import type { PacketStatus } from '#lib/api';

	export interface PacketStatusBadgeProps {
		status: PacketStatus;
	}
</script>

<script lang="ts">
	import { Badge } from '#lib/components/ui';
	import { PACKET_STATUS_LABELS, PACKET_STATUS_TONES } from '#lib/packets/api.js';

	/** A packet's status: `Ready for approval`, `Needs review` or `Dismissed`. */
	let { status }: PacketStatusBadgeProps = $props();
</script>

<Badge tone={PACKET_STATUS_TONES[status]}>{PACKET_STATUS_LABELS[status]}</Badge>
