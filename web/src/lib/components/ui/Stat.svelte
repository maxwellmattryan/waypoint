<script lang="ts" module>
	import type { HTMLAttributes } from 'svelte/elements';
	import type { BadgeTone } from './styles';

	export interface StatProps extends HTMLAttributes<HTMLDivElement> {
		label: string;
		value: string;
		/** A smaller line under the value. */
		detail?: string;
		/** Colours the value; `neutral` (the default) keeps the body text colour. */
		tone?: BadgeTone;
	}
</script>

<script lang="ts">
	import { statTones } from './styles';

	let { label, value, detail, tone = 'neutral', class: className, ...rest }: StatProps = $props();
</script>

<div
	class={['flex min-w-0 flex-col gap-1 rounded-lg border border-border bg-surface p-4', className]}
	{...rest}
>
	<p class="text-xs font-medium text-fg-muted">{label}</p>
	<p class={['truncate text-xl font-semibold tabular-nums', statTones[tone]]}>{value}</p>
	{#if detail}
		<p class="truncate text-xs text-fg-subtle">{detail}</p>
	{/if}
</div>
