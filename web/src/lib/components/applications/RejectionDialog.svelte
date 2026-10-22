<script lang="ts" module>
	export interface RejectionDialogProps {
		/** Bindable. */
		open?: boolean;
		/**
		 * `status`: confirms moving to Rejected, with optional feedback. `feedback-only`: the
		 * application is already rejected and feedback is required.
		 */
		mode: 'status' | 'feedback-only';
		/** Text under the title, e.g. "Acme · Staff Engineer". */
		description?: string;
		/** Sends the trimmed feedback (`""` for none); resolves `true` on success, which closes the dialog. */
		submit: (feedback: string) => Promise<boolean>;
		/** Called when the dialog closes without sending: Cancel, Escape, the Close button or the backdrop. */
		oncancel?: () => void;
	}
</script>

<script lang="ts">
	import { Button, Dialog, Textarea } from '#lib/components/ui';

	let { open = $bindable(false), mode, description, submit, oncancel }: RejectionDialogProps =
		$props();

	let feedback = $state('');
	let sending = $state(false);
	const feedbackOnly = $derived(mode === 'feedback-only');
	const blank = $derived(feedback.trim() === '');

	function cancelled() {
		feedback = '';
		oncancel?.();
	}

	function cancel() {
		open = false;
		cancelled();
	}

	async function confirm() {
		if (sending || (feedbackOnly && blank)) return;
		sending = true;
		const ok = await submit(feedback.trim());
		sending = false;
		if (ok) {
			open = false;
			feedback = '';
		}
	}
</script>

<Dialog
	bind:open
	title={feedbackOnly ? 'Add rejection feedback' : 'Mark as rejected?'}
	{description}
	onclose={cancelled}
>
	<Textarea
		label={feedbackOnly ? 'Feedback' : 'Feedback (optional)'}
		hint={feedbackOnly ? undefined : 'What they said, if anything'}
		bind:value={feedback}
		rows={4}
		maxlength={20000}
		required={feedbackOnly}
	/>
	{#snippet footer()}
		<Button variant="secondary" onclick={cancel}>Cancel</Button>
		<Button
			variant={feedbackOnly ? 'primary' : 'danger'}
			disabled={sending || (feedbackOnly && blank)}
			onclick={confirm}
		>
			{#if sending}
				Saving…
			{:else}
				{feedbackOnly ? 'Save feedback' : 'Mark rejected'}
			{/if}
		</Button>
	{/snippet}
</Dialog>
