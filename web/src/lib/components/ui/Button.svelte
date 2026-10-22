<script lang="ts" module>
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import type { ButtonSize, ButtonVariant } from './styles';

	export interface ButtonProps extends HTMLButtonAttributes {
		variant?: ButtonVariant;
		size?: ButtonSize;
	}
</script>

<script lang="ts">
	import { buttonSizes, buttonVariants, focusRing, interactive } from './styles';

	let {
		variant = 'primary',
		size = 'md',
		type = 'button',
		class: className,
		children,
		...rest
	}: ButtonProps = $props();
</script>

<button
	{type}
	class={[
		'inline-flex items-center justify-center gap-2 rounded-md font-medium',
		interactive,
		focusRing,
		buttonSizes[size],
		buttonVariants[variant],
		className
	]}
	{...rest}
>
	{@render children?.()}
</button>
