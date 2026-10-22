<script lang="ts" module>
	import type { BadgeTone } from '#lib/components/ui';

	export interface StatusRowProps {
		label: string;
		tone: BadgeTone;
		value: string;
	}
</script>

<script lang="ts">
	import { Badge } from '#lib/components/ui';

	let { label, tone, value }: StatusRowProps = $props();
</script>

<!-- One `dt`/`dd` pair: a name and its status badge. Use inside a `dl`. -->
<div class="flex items-center justify-between gap-4">
	<dt class="text-sm text-fg-muted">{label}</dt>
	<dd><Badge {tone}>{value}</Badge></dd>
</div>
