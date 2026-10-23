<script lang="ts" module>
	export interface TagProps {
		/** The tag's text. */
		label: string;
		/** Makes the tag a button, e.g. to filter by it. */
		onclick?: () => void;
		/** Adds a trailing "Remove tag <label>" button. */
		onremove?: () => void;
		disabled?: boolean;
		class?: string;
	}
</script>

<script lang="ts">
	import CloseIcon from './CloseIcon.svelte';
	import IconButton from './IconButton.svelte';
	import { focusRing, interactive, tagBase, tagPressable } from './styles';

	let { label, onclick, onremove, disabled = false, class: className }: TagProps = $props();
</script>

{#snippet text()}
	<span class="truncate">{label}</span>
{/snippet}

{#if onremove}
	<span class={[tagBase, 'pr-0', className]}>
		{#if onclick}
			<button
				type="button"
				class={['rounded-full enabled:hover:underline enabled:active:opacity-75', interactive, focusRing]}
				{disabled}
				{onclick}
			>
				{@render text()}
			</button>
		{:else}
			{@render text()}
		{/if}
		<IconButton
			size="xs"
			label="Remove tag {label}"
			class="rounded-full"
			{disabled}
			onclick={onremove}
		>
			<CloseIcon />
		</IconButton>
	</span>
{:else if onclick}
	<button
		type="button"
		class={[tagBase, interactive, focusRing, tagPressable, className]}
		{disabled}
		{onclick}
	>
		{@render text()}
	</button>
{:else}
	<span class={[tagBase, className]}>{@render text()}</span>
{/if}
