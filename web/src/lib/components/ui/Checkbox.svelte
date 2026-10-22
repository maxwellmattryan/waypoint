<script lang="ts" module>
	import type { HTMLInputAttributes } from 'svelte/elements';

	export interface CheckboxProps extends Omit<HTMLInputAttributes, 'type' | 'checked'> {
		label: string;
		checked?: boolean;
	}
</script>

<script lang="ts">
	import { focusRing } from './styles';

	let {
		label,
		checked = $bindable(false),
		disabled = false,
		class: className,
		...rest
	}: CheckboxProps = $props();
</script>

<label
	class={[
		'inline-flex items-center gap-2 text-sm text-fg',
		disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'
	]}
>
	<input
		type="checkbox"
		bind:checked
		{disabled}
		class={[
			'size-4 shrink-0 rounded-sm accent-primary cursor-pointer disabled:cursor-not-allowed transition-colors',
			'enabled:hover:accent-primary-hover enabled:active:accent-primary-active',
			focusRing,
			className
		]}
		{...rest}
	/>
	<span>{label}</span>
</label>
