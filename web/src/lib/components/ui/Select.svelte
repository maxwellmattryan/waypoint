<script lang="ts" module>
	import type { HTMLSelectAttributes } from 'svelte/elements';

	export interface SelectOption {
		value: string;
		label: string;
		disabled?: boolean;
	}

	export interface SelectProps extends Omit<HTMLSelectAttributes, 'value' | 'id'> {
		label: string;
		/** Control id; generated when omitted. */
		id?: string;
		options: SelectOption[];
		/** Rendered as a disabled, empty-value first option. */
		placeholder?: string;
		hint?: string;
		error?: string;
		value?: string;
	}
</script>

<script lang="ts">
	import Field from './Field.svelte';
	import { controlBase, controlBorder, describedBy, focusRing } from './styles';

	const uid = $props.id();

	let {
		label,
		options,
		placeholder,
		hint,
		error,
		id = uid,
		value = $bindable(placeholder === undefined ? undefined : ''),
		class: className,
		...rest
	}: SelectProps = $props();
</script>

<Field {id} {label} {hint} {error}>
	<div class="relative">
		<select
			{id}
			bind:value
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy(id, hint, error)}
			class={[
				'h-10 appearance-none pr-10 pl-3 cursor-pointer',
				controlBase,
				controlBorder(!!error),
				focusRing,
				className
			]}
			{...rest}
		>
			{#if placeholder !== undefined}
				<option value="" disabled>{placeholder}</option>
			{/if}
			{#each options as option (option.value)}
				<option value={option.value} disabled={option.disabled}>{option.label}</option>
			{/each}
		</select>
		<svg
			class="pointer-events-none absolute top-3 right-3 size-4 text-fg-muted"
			viewBox="0 0 16 16"
			fill="none"
			stroke="currentColor"
			stroke-width="1.5"
			aria-hidden="true"
		>
			<path d="M4 6l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" />
		</svg>
	</div>
</Field>
