<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export interface ConfirmDialogProps {
		/** Whether the dialog is shown. Bindable. */
		open?: boolean;
		title: string;
		description?: string;
		/** The confirm button's text. Default "Delete". */
		confirmLabel?: string;
		/** The confirm button's text while `onconfirm` runs. Default "Deleting…". */
		pendingLabel?: string;
		/** `danger` (default) or `primary` confirm button. */
		tone?: 'danger' | 'primary';
		/** Runs the action; resolving `true` closes the dialog, `false` keeps it open. */
		onconfirm: () => Promise<boolean>;
		/** Optional body, e.g. an error from the last attempt. */
		children?: Snippet;
	}
</script>

<script lang="ts">
	import Button from './Button.svelte';
	import Dialog from './Dialog.svelte';

	let {
		open = $bindable(false),
		title,
		description,
		confirmLabel = 'Delete',
		pendingLabel = 'Deleting…',
		tone = 'danger',
		onconfirm,
		children
	}: ConfirmDialogProps = $props();

	let pending = $state(false);

	async function confirm() {
		if (pending) return;
		pending = true;
		try {
			if (await onconfirm()) open = false;
		} finally {
			pending = false;
		}
	}
</script>

<Dialog bind:open {title} {description} size="sm">
	{@render children?.()}
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button variant={tone} disabled={pending} aria-busy={pending || undefined} onclick={confirm}>
			{pending ? pendingLabel : confirmLabel}
		</Button>
	{/snippet}
</Dialog>
