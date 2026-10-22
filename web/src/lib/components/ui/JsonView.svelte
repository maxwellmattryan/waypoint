<script lang="ts" module>
	import type { HTMLAttributes } from 'svelte/elements';

	export interface JsonViewProps extends HTMLAttributes<HTMLElement> {
		value: unknown;
		/** Names the block for assistive tech (`aria-label`). */
		label: string;
		/** Shown instead of the block for `null` or `undefined`. */
		empty?: string;
	}
</script>

<script lang="ts">
	import { focusRing } from './styles';

	let { value, label, empty = 'None', class: className, ...rest }: JsonViewProps = $props();

	// Text only, never markup: a string holding HTML renders as its literal characters.
	const text = $derived(value === null || value === undefined ? null : JSON.stringify(value, null, 2));
</script>

{#if text === null}
	<p class={['text-sm text-fg-subtle', className]} {...rest}>{empty}</p>
{:else}
	<!-- Focusable so keyboard users can scroll a long value. -->
	<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
	<pre
		tabindex="0"
		aria-label={label}
		class={[
			'max-h-80 overflow-auto rounded-md border border-border bg-surface-raised p-3 font-mono text-xs break-words whitespace-pre-wrap text-fg',
			focusRing,
			className
		]}
		{...rest}>{text}</pre>
{/if}
