<script lang="ts" module>
	import type { HTMLAnchorAttributes, HTMLAttributes } from 'svelte/elements';

	export type CardProps = Omit<HTMLAttributes<HTMLDivElement>, 'draggable'> &
		Pick<HTMLAnchorAttributes, 'href' | 'target' | 'rel'> & {
			/**
			 * Makes the card a drag source with the grab cursor. Ignored when `href` is set:
			 * a link card stays a link.
			 */
			draggable?: boolean;
		};
</script>

<script lang="ts">
	import { focusRing } from './styles';

	let { href, draggable = false, class: className, children, ...rest }: CardProps = $props();

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
	<!-- One element either way, so toggling `draggable` keeps the node (and its focus). -->
	<div
		draggable={draggable ? 'true' : undefined}
		class={[
			base,
			draggable && 'cursor-grab transition-colors hover:border-border-strong active:cursor-grabbing',
			className
		]}
		{...rest}
	>
		{@render children?.()}
	</div>
{/if}
