<script lang="ts">
	import type { Snippet } from 'svelte';

	interface FieldProps {
		/** Id of the wrapped control. Hint and error ids derive from it. */
		id: string;
		label: string;
		hint?: string;
		error?: string;
		children: Snippet;
	}

	let { id, label, hint, error, children }: FieldProps = $props();
</script>

<div class="flex flex-col gap-1">
	<label for={id} class="text-sm font-medium text-fg">{label}</label>
	{@render children()}
	{#if hint}
		<p id="{id}-hint" class="text-xs text-fg-muted">{hint}</p>
	{/if}
	{#if error}
		<p id="{id}-error" class="text-xs text-danger">{error}</p>
	{/if}
</div>
