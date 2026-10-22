<script lang="ts" module>
	export interface DeleteAllDataDialogProps {
		/** Bindable. */
		open?: boolean;
		/** Sends the typed phrase; resolves `true` on success, which closes the dialog. */
		submit: (confirmation: string) => Promise<boolean>;
	}
</script>

<script lang="ts">
	import { Button, Dialog, Input } from '#lib/components/ui';
	import { DELETE_CONFIRMATION } from '#lib/dashboard/kill-switch.js';

	let { open = $bindable(false), submit }: DeleteAllDataDialogProps = $props();

	let typed = $state('');
	let sending = $state(false);
	const confirmed = $derived(typed === DELETE_CONFIRMATION);

	function cancel() {
		open = false;
		typed = '';
	}

	async function confirm() {
		if (sending || !confirmed) return;
		sending = true;
		const ok = await submit(typed);
		sending = false;
		if (ok) {
			open = false;
			typed = '';
		}
	}
</script>

<Dialog
	bind:open
	title="Delete all data?"
	description="Every application, resume, run, job and model-call log is deleted, and Waypoint's Google access is revoked. Agents stay paused afterwards. This can't be undone."
	onclose={() => (typed = '')}
>
	<Input
		label={`Type "${DELETE_CONFIRMATION}" to confirm`}
		bind:value={typed}
		autocomplete="off"
		spellcheck={false}
	/>
	{#snippet footer()}
		<Button variant="secondary" onclick={cancel}>Cancel</Button>
		<Button variant="danger" disabled={sending || !confirmed} onclick={confirm}>
			{sending ? 'Deleting…' : 'Delete all data'}
		</Button>
	{/snippet}
</Dialog>
