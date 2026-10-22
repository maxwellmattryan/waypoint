<script lang="ts" module>
	import type { HTMLAttributes } from 'svelte/elements';
	import type { BadgeTone } from './styles';

	export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
		tone?: BadgeTone;
	}
</script>

<script lang="ts">
	import { badgeTones } from './styles';

	let { tone = 'neutral', class: className, children, ...rest }: BadgeProps = $props();
</script>

<span
	class={[
		'inline-flex h-6 items-center rounded-full px-2 text-xs font-medium whitespace-nowrap',
		badgeTones[tone],
		className
	]}
	{...rest}
>
	{@render children?.()}
</span>
