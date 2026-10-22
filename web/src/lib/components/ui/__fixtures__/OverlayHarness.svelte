<script lang="ts">
	import { Button, Dialog, Drawer } from '../index';

	let {
		kind,
		closeOnBackdrop = true,
		autofocus = false,
		onclose
	}: {
		kind: 'dialog' | 'drawer';
		closeOnBackdrop?: boolean;
		/** Put `data-autofocus` on the "Last" button. */
		autofocus?: boolean;
		onclose?: () => void;
	} = $props();

	let open = $state(false);
</script>

<Button onclick={() => (open = true)}>Open</Button>
<span data-testid="state" data-open={open}></span>

{#snippet body()}
	<Button variant="secondary">First</Button>
	<Button variant="secondary" data-autofocus={autofocus || undefined}>Last</Button>
{/snippet}

{#if kind === 'dialog'}
	<Dialog bind:open title="Edit application" {closeOnBackdrop} {onclose}>{@render body()}</Dialog>
{:else}
	<Drawer bind:open title="Edit application" {closeOnBackdrop} {onclose}>{@render body()}</Drawer>
{/if}
