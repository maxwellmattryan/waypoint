<script lang="ts" module>
	import type { HTMLAnchorAttributes } from 'svelte/elements';

	export interface NavLinkProps extends HTMLAnchorAttributes {
		/** This link's section is the one shown: sets `aria-current="page"` and the active style. */
		current?: boolean;
		/** Render without `href`, out of the tab order. */
		disabled?: boolean;
	}
</script>

<script lang="ts">
	import { focusRing } from './styles';

	let {
		href,
		current = false,
		disabled = false,
		class: className,
		children,
		...rest
	}: NavLinkProps = $props();
</script>

<!-- A primitive forwards the caller's href; callers resolve internal routes before passing them. -->
<!-- eslint-disable svelte/no-navigation-without-resolve -->
<a
	href={disabled ? undefined : href}
	aria-current={current ? 'page' : undefined}
	aria-disabled={disabled ? 'true' : undefined}
	tabindex={disabled ? -1 : undefined}
	class={[
		'rounded-md px-3 py-2 text-sm font-medium transition-colors',
		focusRing,
		disabled
			? 'cursor-not-allowed text-fg-subtle opacity-50'
			: [
					'cursor-pointer hover:bg-surface-raised hover:text-fg active:bg-border',
					current ? 'bg-surface-raised text-fg' : 'text-fg-muted'
				],
		className
	]}
	{...rest}
>
	{@render children?.()}
</a>
<!-- eslint-enable svelte/no-navigation-without-resolve -->
