<script lang="ts">
	import { Button, Tooltip } from '../index';

	let { delay }: { delay?: number } = $props();
</script>

<Tooltip content="Archive application" {delay}>
	{#snippet children(t)}
		<Button {...t}>Archive</Button>
	{/snippet}
</Tooltip>
