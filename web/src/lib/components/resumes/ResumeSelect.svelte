<script lang="ts" module>
	import type { ResumeVersion } from '#lib/api';
	import type { SelectProps } from '#lib/components/ui';

	export interface ResumeSelectProps extends Omit<SelectProps, 'options' | 'value' | 'label'> {
		/** Default "Resume sent". */
		label?: string;
		/** Every version, in the order to list them (newest first). */
		versions: ResumeVersion[];
		/** The linked version's id, or `null` for none. Bindable. */
		value?: number | null;
	}
</script>

<script lang="ts">
	import { Select } from '#lib/components/ui';

	let {
		label = 'Resume sent',
		versions,
		value = $bindable(null),
		...rest
	}: ResumeSelectProps = $props();

	const NONE = '';

	const options = $derived([
		{ value: NONE, label: 'None' },
		...versions.map((version) => ({ value: String(version.id), label: version.name }))
	]);
</script>

<Select
	{label}
	{options}
	bind:value={
		() => (value === null ? NONE : String(value)),
		(next) => {
			value = next && /^\d+$/.test(next) ? Number(next) : null;
		}
	}
	{...rest}
/>
