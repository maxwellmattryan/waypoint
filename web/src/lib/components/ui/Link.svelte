<script lang="ts" module>
	import type { HTMLAnchorAttributes } from 'svelte/elements';

	export interface LinkProps extends HTMLAnchorAttributes {
		/** Open in a new tab with `rel="noopener noreferrer"`. */
		external?: boolean;
		/** Render without `href`, out of the tab order. */
		disabled?: boolean;
	}
</script>

<script lang="ts">
	import { focusRing } from './styles';

	let {
		href,
		external = false,
		disabled = false,
		class: className,
		children,
		...rest
	}: LinkProps = $props();
</script>

<!-- A primitive forwards the caller's href; callers resolve internal routes before passing them. -->
<!-- eslint-disable svelte/no-navigation-without-resolve -->
<a
	href={disabled ? undefined : href}
	target={external && !disabled ? '_blank' : undefined}
	rel={external && !disabled ? 'noopener noreferrer' : undefined}
	aria-disabled={disabled ? 'true' : undefined}
	tabindex={disabled ? -1 : undefined}
	class={[
		'rounded-sm font-medium underline-offset-4 transition-colors',
		focusRing,
		disabled
			? 'cursor-not-allowed text-fg-subtle opacity-50'
			: 'cursor-pointer text-primary hover:text-primary-hover hover:underline active:text-primary-active',
		className
	]}
	{...rest}
>
	{@render children?.()}
</a>
<!-- eslint-enable svelte/no-navigation-without-resolve -->
