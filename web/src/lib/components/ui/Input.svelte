<script lang="ts" module>
	import type { HTMLInputAttributes } from 'svelte/elements';

	export interface InputProps extends Omit<HTMLInputAttributes, 'value' | 'id'> {
		label: string;
		/** Control id; generated when omitted. */
		id?: string;
		hint?: string;
		error?: string;
		value?: string | number | null;
	}
</script>

<script lang="ts">
	import Field from './Field.svelte';
	import { controlBase, controlBorder, describedBy, focusRing } from './styles';

	const uid = $props.id();

	let {
		label,
		hint,
		error,
		id = uid,
		value = $bindable(),
		class: className,
		...rest
	}: InputProps = $props();
</script>

<Field {id} {label} {hint} {error}>
	<input
		{id}
		bind:value
		aria-invalid={error ? 'true' : undefined}
		aria-describedby={describedBy(id, hint, error)}
		class={['h-10 px-3', controlBase, controlBorder(!!error), focusRing, className]}
		{...rest}
	/>
</Field>
