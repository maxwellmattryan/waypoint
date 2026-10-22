<script lang="ts" module>
	import type { HTMLInputAttributes } from 'svelte/elements';

	export interface FileInputProps
		extends Omit<HTMLInputAttributes, 'type' | 'value' | 'id' | 'files' | 'onchange' | 'multiple'> {
		label: string;
		/** Control id; generated when omitted. */
		id?: string;
		hint?: string;
		error?: string;
		/** Text on the button part of the control. Default "Choose file". */
		buttonLabel?: string;
		/** Called with the picked file, or `null` when the pick is cleared. */
		onchange?: (file: File | null) => void;
	}
</script>

<script lang="ts">
	import Field from './Field.svelte';
	import { controlBorder, describedBy, focusRing } from './styles';

	const uid = $props.id();

	let {
		label,
		hint,
		error,
		id = uid,
		buttonLabel = 'Choose file',
		disabled,
		onchange,
		class: className,
		...rest
	}: FileInputProps = $props();

	let fileName = $state<string | null>(null);

	function picked(event: Event & { currentTarget: HTMLInputElement }) {
		const file = event.currentTarget.files?.[0] ?? null;
		fileName = file?.name ?? null;
		onchange?.(file);
	}
</script>

<Field {id} {label} {hint} {error}>
	<!-- The native input is the control (keyboard, focus, picker); its own button and text are
	     hidden and redrawn underneath so the label reads the same in every browser. -->
	<div class="relative">
		<input
			{id}
			type="file"
			{disabled}
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={describedBy(id, hint, error)}
			onchange={picked}
			class={[
				'peer block h-10 w-full rounded-md border bg-surface text-sm text-transparent file:hidden',
				'cursor-pointer transition-colors disabled:cursor-not-allowed disabled:opacity-50',
				'enabled:hover:bg-surface-raised enabled:active:bg-border',
				controlBorder(!!error),
				focusRing,
				className
			]}
			{...rest}
		/>
		<span
			aria-hidden="true"
			class="pointer-events-none absolute inset-y-0 right-3 left-1 flex items-center gap-3 text-sm peer-disabled:opacity-50"
		>
			<span
				class="inline-flex h-8 shrink-0 items-center rounded-sm border border-border-strong bg-surface-raised px-3 font-medium text-fg"
			>
				{buttonLabel}
			</span>
			<span class={['truncate', fileName ? 'text-fg' : 'text-fg-muted']}>
				{fileName ?? 'No file chosen'}
			</span>
		</span>
	</div>
</Field>
