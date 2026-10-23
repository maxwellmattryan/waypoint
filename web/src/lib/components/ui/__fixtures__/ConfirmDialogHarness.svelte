<script lang="ts">
	import { ConfirmDialog } from '../index';

	let { onconfirm }: { onconfirm: () => Promise<boolean> } = $props();

	let open = $state(true);
</script>

<ConfirmDialog
	bind:open
	title="Delete memory"
	description="Waypoint will forget this. This can't be undone."
	{onconfirm}
/>
<span data-testid="state" data-open={open}></span>
