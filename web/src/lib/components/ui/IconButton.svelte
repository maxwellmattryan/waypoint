<script lang="ts" module>
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import type { ButtonVariant, IconButtonSize } from './styles';

	export interface IconButtonProps extends HTMLButtonAttributes {
		/** Accessible name, rendered as `aria-label`. */
		label: string;
		variant?: ButtonVariant;
		/** `xs` (size-5) for buttons inside small controls, `sm` (size-8) or `md` (size-10). */
		size?: IconButtonSize;
	}
</script>

<script lang="ts">
	import { buttonVariants, focusRing, iconButtonSizes, interactive } from './styles';

	let {
		label,
		variant = 'ghost',
		size = 'md',
		type = 'button',
		class: className,
		children,
		...rest
	}: IconButtonProps = $props();
</script>

<button
	{type}
	aria-label={label}
	class={[
		'inline-flex shrink-0 items-center justify-center rounded-md',
		interactive,
		focusRing,
		iconButtonSizes[size],
		buttonVariants[variant],
		className
	]}
	{...rest}
>
	{@render children?.()}
</button>
