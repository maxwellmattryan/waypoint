<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export interface NoteComposerProps {
		/** Stores the (trimmed, non-blank) note; resolves `true` on success, which clears the text. */
		send: (body: string) => Promise<boolean>;
		/** More buttons, rendered before "Add note". */
		actions?: Snippet;
	}
</script>

<script lang="ts">
	import { Button, Textarea } from '#lib/components/ui';

	let { send, actions }: NoteComposerProps = $props();

	let body = $state('');
	let sending = $state(false);
	const blank = $derived(body.trim() === '');

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (blank || sending) return;
		sending = true;
		const ok = await send(body.trim());
		sending = false;
		if (ok) body = '';
	}
</script>

<form class="flex flex-col gap-2" onsubmit={submit}>
	<Textarea label="Add a note" bind:value={body} rows={3} maxlength={20000} />
	<div class="flex flex-wrap justify-end gap-2">
		{@render actions?.()}
		<Button type="submit" disabled={blank || sending}>{sending ? 'Adding…' : 'Add note'}</Button>
	</div>
</form>
