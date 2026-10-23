<script lang="ts" module>
	import type { Run, TidyRequest } from '#lib/api';
	import type { Result } from '#lib/applications/api.js';

	export interface TidyDialogProps {
		/** Bindable. */
		open?: boolean;
		/** Starts the run; `ok` closes the dialog, `invalid` maps its errors to the fields. */
		onstart: (body: TidyRequest) => Promise<Result<Run>>;
	}

	type FieldName = 'note' | 'message' | 'confidence';
</script>

<script lang="ts">
	import { Button, Dialog, Input, Textarea, toast } from '#lib/components/ui';

	let { open = $bindable(false), onstart }: TidyDialogProps = $props();

	const formId = $props.id();
	let note = $state('');
	let message = $state('');
	let confidence = $state<string | number | null>(1);
	let errors = $state<Partial<Record<FieldName, string>>>({});
	let sending = $state(false);

	const RANGE = 'Enter a number from 0 to 1';

	function reset() {
		note = '';
		message = '';
		confidence = 1;
		errors = {};
	}

	/** The confidence as a number from 0 to 1, or `null` for anything else. */
	function parseConfidence(value: string | number | null): number | null {
		if (value === null || value === '') return null;
		const number = typeof value === 'number' ? value : Number(value);
		return Number.isFinite(number) && number >= 0 && number <= 1 ? number : null;
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (sending) return;
		const body = { note: note.trim(), message: message.trim() };
		const level = parseConfidence(confidence);
		const found: Partial<Record<FieldName, string>> = {};
		if (body.note === '') found.note = 'Required';
		if (body.message === '') found.message = 'Required';
		if (level === null) found.confidence = RANGE;
		errors = found;
		if (Object.keys(found).length > 0 || level === null) return;

		sending = true;
		const result = await onstart({ ...body, confidence: level });
		sending = false;
		switch (result.kind) {
			case 'ok':
				open = false;
				reset();
				toast.success(`Run #${result.data.id} started`, {
					description: 'Its draft will wait for your approval in Review.'
				});
				break;
			case 'invalid':
				errors = Object.fromEntries(
					result.errors
						.filter((e) => e.field === 'note' || e.field === 'message' || e.field === 'confidence')
						.map((e) => [e.field, e.message])
				);
				break;
			case 'not-found':
				toast.error('This application no longer exists');
				break;
			default:
				toast.error("Couldn't start the run");
		}
	}
</script>

<Dialog
	bind:open
	title="Run sample workflow"
	description="Adds a note to this application's timeline, then drafts a follow-up that waits for your approval in Review. Nothing is sent."
	onclose={reset}
>
	<form id={formId} novalidate class="flex flex-col gap-4" onsubmit={submit}>
		<Textarea label="Note" bind:value={note} error={errors.note} required maxlength={20000} />
		<Textarea
			label="Follow-up message"
			bind:value={message}
			error={errors.message}
			required
			maxlength={20000}
		/>
		<Input
			label="Confidence"
			type="number"
			min={0}
			max={1}
			step={0.1}
			bind:value={confidence}
			hint="Below 0.8, the note goes to Review instead of the timeline."
			error={errors.confidence}
		/>
	</form>
	{#snippet footer()}
		<Button
			variant="secondary"
			onclick={() => {
				open = false;
				reset();
			}}
		>
			Cancel
		</Button>
		<Button type="submit" form={formId} disabled={sending}>
			{sending ? 'Starting…' : 'Start run'}
		</Button>
	{/snippet}
</Dialog>
