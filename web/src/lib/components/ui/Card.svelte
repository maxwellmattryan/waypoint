<script lang="ts" module>
	import type { HTMLAnchorAttributes, HTMLAttributes } from 'svelte/elements';

	export type CardProps = HTMLAttributes<HTMLDivElement> &
		Pick<HTMLAnchorAttributes, 'href' | 'target' | 'rel'>;
</script>

<script lang="ts">
	import { focusRing } from './styles';

	let { href, class: className, children, ...rest }: CardProps = $props();

	const base = 'block rounded-lg border border-border bg-surface p-4 text-fg shadow-sm';
</script>

{#if href}
	<!-- A primitive forwards the caller's href; callers resolve internal routes before passing them. -->
	<!-- eslint-disable svelte/no-navigation-without-resolve -->
	<a
		{href}
		class={[
			base,
			'cursor-pointer transition-colors hover:border-border-strong hover:bg-surface-raised active:bg-border',
			'aria-disabled:cursor-not-allowed aria-disabled:opacity-50',
			focusRing,
			className
		]}
		{...rest as HTMLAnchorAttributes}
	>
		{@render children?.()}
	</a>
	<!-- eslint-enable svelte/no-navigation-without-resolve -->
{:else}
	<div class={[base, className]} {...rest}>
		{@render children?.()}
	</div>
{/if}
