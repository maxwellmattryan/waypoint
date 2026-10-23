<script lang="ts" module>
	export interface RejectActionDialogProps {
		/** Bindable. */
		open?: boolean;
		/** A rejection is in flight; the confirm button is disabled. */
		busy?: boolean;
		/** Called when the user confirms; the dialog closes at once. */
		onconfirm: () => void;
	}
</script>

<script lang="ts">
	import { Button, Dialog } from '#lib/components/ui';

	let { open = $bindable(false), busy = false, onconfirm }: RejectActionDialogProps = $props();

	function confirm() {
		open = false;
		onconfirm();
	}
</script>

<Dialog
	bind:open
	title="Reject this action?"
	description="The run ends as rejected and can't be resumed. Any other actions it is waiting on are rejected too."
	size="sm"
>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button variant="danger" disabled={busy} onclick={confirm}>Reject</Button>
	{/snippet}
</Dialog>
