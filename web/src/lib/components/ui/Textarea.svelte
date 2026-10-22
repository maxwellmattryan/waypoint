<script lang="ts" module>
	import type { HTMLTextareaAttributes } from 'svelte/elements';

	export interface TextareaProps extends Omit<HTMLTextareaAttributes, 'value' | 'id'> {
		label: string;
		/** Control id; generated when omitted. */
		id?: string;
		hint?: string;
		error?: string;
		value?: string | null;
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
		rows = 3,
		value = $bindable(),
		class: className,
		...rest
	}: TextareaProps = $props();
</script>

<Field {id} {label} {hint} {error}>
	<textarea
		{id}
		{rows}
		bind:value
		aria-invalid={error ? 'true' : undefined}
		aria-describedby={describedBy(id, hint, error)}
		class={['px-3 py-2', controlBase, controlBorder(!!error), focusRing, className]}
		{...rest}
	></textarea>
</Field>
