<script lang="ts">
	import { Button, Dialog, Drawer } from '../index';

	let drawerOpen = $state(false);
	let dialogOpen = $state(false);
</script>

<Button onclick={() => (drawerOpen = true)}>Open drawer</Button>

<Drawer bind:open={drawerOpen} title="Application">
	<Button onclick={() => (dialogOpen = true)}>Open dialog</Button>
	<Dialog bind:open={dialogOpen} title="Confirm">
		<Button variant="secondary">Okay</Button>
	</Dialog>
</Drawer>
