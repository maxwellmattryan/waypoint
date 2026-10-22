<script lang="ts">
	import { Tabs, type TabItem } from '../index';

	let { items, value: initial }: { items: TabItem[]; value?: string } = $props();

	// svelte-ignore state_referenced_locally
	let value = $state(initial);
</script>

<Tabs label="Application" {items} bind:value>
	{#snippet children(active)}
		<p>Panel {active}</p>
	{/snippet}
</Tabs>
<output data-testid="value">{value ?? ''}</output>
