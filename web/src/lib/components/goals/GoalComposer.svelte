<script lang="ts" module>
	export interface GoalComposerProps {
		/** A goal is being sent; the button reads "Planning…" and is disabled. */
		busy?: boolean;
		/** The API's message for the text, shown under the field. */
		error?: string | null;
		/** Sends the goal; resolves `true` once it was created, which clears the field. */
		onsubmit: (text: string) => Promise<boolean> | boolean;
	}
</script>

<script lang="ts">
	import { Button, Textarea } from '#lib/components/ui';
	import { MAX_GOAL } from '#lib/goals/goals.svelte.js';

	/** The goal composer: what the user wants, in their own words, and **Plan it**. */
	let { busy = false, error = null, onsubmit }: GoalComposerProps = $props();

	const formId = $props.id();
	let text = $state('');
	const length = $derived([...text.trim()].length);
	const tooLong = $derived(length > MAX_GOAL);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy || length === 0 || tooLong) return;
		if (await onsubmit(text)) text = '';
	}
</script>

<form id={formId} class="flex flex-col gap-4" aria-label="New goal" onsubmit={submit} novalidate>
	<Textarea
		label="What are you looking for?"
		hint={tooLong
			? `At most ${MAX_GOAL} characters`
			: 'e.g. Senior frontend roles at climate companies in NYC, apply to 5 this week'}
		rows={3}
		bind:value={text}
		disabled={busy}
		error={error ?? undefined}
	/>
	<Button type="submit" class="self-end" disabled={busy || length === 0 || tooLong}>
		{busy ? 'Planning…' : 'Plan it'}
	</Button>
</form>
