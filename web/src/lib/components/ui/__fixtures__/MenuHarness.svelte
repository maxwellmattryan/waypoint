<script lang="ts">
	import { Button, Dialog, Menu, type MenuItem } from '../index';

	let {
		onselect = () => {},
		inDialog = false,
		disabled = false,
		items = [
			{ value: 'applied', label: 'Applied' },
			{ value: 'offer', label: 'Offer', disabled: true },
			{ value: 'interviewing', label: 'Interviewing' },
			{ value: 'rejected', label: 'Rejected' }
		]
	}: {
		onselect?: (value: string) => void;
		/** Render the menu inside an open Dialog. */
		inDialog?: boolean;
		/** Disable the trigger. */
		disabled?: boolean;
		items?: MenuItem[];
	} = $props();

	let open = $state(true);
</script>

{#snippet menu()}
	<Menu label="Move to" heading="Move to" {items} {onselect}>
		{#snippet trigger(t)}
			<Button {...t} {disabled}>Move</Button>
		{/snippet}
	</Menu>
{/snippet}

<Button>Outside</Button>
{#if inDialog}
	<span data-testid="dialog-state" data-open={open}></span>
	<Dialog bind:open title="Edit application">{@render menu()}</Dialog>
{:else}
	{@render menu()}
{/if}
