<script lang="ts" module>
	export interface ReplanDialogProps {
		/** Bindable. */
		open?: boolean;
		/** A replan request is in flight. */
		busy?: boolean;
		/** Asks for a replan; resolves `true` once it was queued, which closes the dialog. */
		onsubmit: (instruction: string) => Promise<boolean> | boolean;
	}
</script>

<script lang="ts">
	import { Button, Dialog, Textarea } from '#lib/components/ui';

	let { open = $bindable(false), busy = false, onsubmit }: ReplanDialogProps = $props();

	const MAX = 1000;
	const formId = $props.id();
	let instruction = $state('');
	const tooLong = $derived([...instruction.trim()].length > MAX);

	$effect(() => {
		if (open) instruction = '';
	});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy || tooLong) return;
		if (await onsubmit(instruction)) open = false;
	}
</script>

<Dialog
	bind:open
	title="Replan"
	description="The planner writes a new version of the plan. Steps whose inputs don't change keep their results."
>
	<form id={formId} onsubmit={submit} aria-label="Replan" novalidate>
		<Textarea
			label="What should change? (optional)"
			hint={tooLong ? undefined : 'e.g. Focus on Series B companies'}
			rows={3}
			bind:value={instruction}
			disabled={busy}
			error={tooLong ? `At most ${MAX} characters` : undefined}
			data-autofocus
		/>
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button type="submit" form={formId} disabled={busy || tooLong}>
			{busy ? 'Asking…' : 'Replan'}
		</Button>
	{/snippet}
</Dialog>
