<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { HTMLAttributes } from 'svelte/elements';

	export interface KeyValueItem {
		key: string;
		/** Text, or a snippet for richer content such as a Badge. */
		value: string | Snippet;
	}

	export interface KeyValueProps extends HTMLAttributes<HTMLDListElement> {
		items: KeyValueItem[];
	}
</script>

<script lang="ts">
	let { items, class: className, ...rest }: KeyValueProps = $props();
</script>

<dl class={['grid grid-cols-[max-content_1fr] gap-x-4 gap-y-2 text-sm', className]} {...rest}>
	{#each items as item (item.key)}
		<dt class="text-fg-muted">{item.key}</dt>
		<dd class="min-w-0 break-words text-fg">
			{#if typeof item.value === 'string'}
				{item.value}
			{:else}
				{@render item.value()}
			{/if}
		</dd>
	{/each}
</dl>
