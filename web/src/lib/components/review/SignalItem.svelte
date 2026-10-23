<script lang="ts" module>
	import type { MemorySignal } from '#lib/api';

	export interface SignalItemProps {
		signal: MemorySignal;
	}
</script>

<script lang="ts">
	import { Card } from '#lib/components/ui';
	import SignalDetails from './SignalDetails.svelte';

	let { signal }: SignalItemProps = $props();
</script>

<Card class="flex flex-col gap-4" data-signal={signal.id}>
	<SignalDetails {signal} />
</Card>
