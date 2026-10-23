<script lang="ts" module>
	import type { PendingAction } from '#lib/api';
	import type { Result } from '#lib/applications/api.js';

	export interface EditActionDialogProps {
		/** Bindable. */
		open?: boolean;
		/** The requested input, prefilled as pretty JSON. */
		input: unknown;
		/** Approves with the edited input; an `ok` result closes the dialog, `invalid` shows its errors. */
		onapprove: (input: Record<string, unknown>) => Promise<Result<PendingAction>>;
	}
</script>

<script lang="ts">
	import { Button, Dialog, Textarea } from '#lib/components/ui';

	let { open = $bindable(false), input, onapprove }: EditActionDialogProps = $props();

	const formId = $props.id();
	let text = $state('');
	let error = $state<string | undefined>();
	let sending = $state(false);

	// Prefill each time the dialog opens.
	$effect.pre(() => {
		if (open) {
			text = JSON.stringify(input, null, 2);
			error = undefined;
		}
	});

	/** The text as a plain JSON object, or the field error that stops it being sent. */
	function parse(source: string): { value: Record<string, unknown> } | { error: string } {
		let value: unknown;
		try {
			value = JSON.parse(source);
		} catch {
			return { error: 'Not valid JSON' };
		}
		if (typeof value !== 'object' || value === null || Array.isArray(value)) {
			return { error: 'Must be a JSON object' };
		}
		return { value: value as Record<string, unknown> };
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (sending) return;
		const parsed = parse(text);
		if ('error' in parsed) {
			error = parsed.error;
			return;
		}
		error = undefined;
		sending = true;
		const result = await onapprove(parsed.value);
		sending = false;
		if (result.kind === 'ok') open = false;
		else if (result.kind === 'invalid') error = result.errors.map((e) => e.message).join(' ');
	}
</script>

<Dialog
	bind:open
	title="Edit before approving"
	description="Change the input, then approve it. The tool checks it before it runs."
	size="lg"
>
	<form id={formId} novalidate onsubmit={submit}>
		<Textarea
			label="Input (JSON)"
			bind:value={text}
			{error}
			rows={10}
			spellcheck={false}
			class="font-mono text-xs"
		/>
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button type="submit" form={formId} disabled={sending}>
			{sending ? 'Approving…' : 'Approve edited'}
		</Button>
	{/snippet}
</Dialog>
