<script lang="ts" module>
	import type { ReviewItem } from '#lib/api';
	import type { Result } from '#lib/applications/api.js';

	export interface DismissReviewItemDialogProps {
		/** Bindable. */
		open?: boolean;
		/** A dismissal is in flight; the Dismiss button is disabled. */
		busy?: boolean;
		/**
		 * Dismisses with the trimmed reason, or none when it's blank. `invalid` keeps the dialog
		 * open with the message under the field; any other result closes it.
		 */
		ondismiss: (reason?: string) => Promise<Result<ReviewItem>>;
	}
</script>

<script lang="ts">
	import { Button, Dialog, Textarea } from '#lib/components/ui';

	/** Most characters the API stores in a reason. */
	const MAX_REASON = 1000;

	let { open = $bindable(false), busy = false, ondismiss }: DismissReviewItemDialogProps = $props();

	const formId = $props.id();
	let text = $state('');
	let error = $state<string | undefined>();
	let sending = $state(false);

	// Start empty each time the dialog opens.
	$effect.pre(() => {
		if (open) {
			text = '';
			error = undefined;
		}
	});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (sending || busy) return;
		const reason = text.trim();
		error = undefined;
		sending = true;
		const result = await ondismiss(reason === '' ? undefined : reason);
		sending = false;
		if (result.kind === 'invalid') error = result.errors.map((e) => e.message).join(' ');
		else open = false;
	}
</script>

<Dialog
	bind:open
	title="Dismiss this result?"
	description="Nothing is applied, and it leaves the queue."
	size="sm"
>
	<form id={formId} novalidate onsubmit={submit}>
		<Textarea
			label="Reason"
			hint="Optional. Saying what was wrong helps Waypoint learn."
			bind:value={text}
			{error}
			rows={3}
			maxlength={MAX_REASON}
			data-autofocus
		/>
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button type="submit" form={formId} disabled={busy || sending}>
			{sending ? 'Dismissing…' : 'Dismiss'}
		</Button>
	{/snippet}
</Dialog>
