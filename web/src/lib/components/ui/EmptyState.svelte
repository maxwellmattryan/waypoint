<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { HTMLAttributes } from 'svelte/elements';

	export interface EmptyStateProps extends Omit<HTMLAttributes<HTMLDivElement>, 'title'> {
		title: string;
		description?: string;
		icon?: Snippet;
		action?: Snippet;
	}
</script>

<script lang="ts">
	let { title, description, icon, action, class: className, ...rest }: EmptyStateProps = $props();
</script>

<div
	class={['flex flex-col items-center justify-center gap-2 py-12 text-center', className]}
	{...rest}
>
	{#if icon}
		<div class="text-fg-subtle">{@render icon()}</div>
	{/if}
	<p class="text-base font-semibold text-fg">{title}</p>
	{#if description}
		<p class="max-w-sm text-sm text-fg-muted">{description}</p>
	{/if}
	{#if action}
		<div class="mt-2">{@render action()}</div>
	{/if}
</div>
