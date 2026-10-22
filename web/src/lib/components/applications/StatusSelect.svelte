<script lang="ts" module>
	import type { ApplicationStatus } from '#lib/api';
	import type { SelectProps } from '#lib/components/ui';

	export interface StatusSelectProps extends Omit<SelectProps, 'options' | 'value' | 'label'> {
		/** Default "Status". */
		label?: string;
		/** Bindable. */
		value?: ApplicationStatus;
	}
</script>

<script lang="ts">
	import { isStatus, statusOptions } from '#lib/applications/statuses.js';
	import { Select } from '#lib/components/ui';

	let { label = 'Status', value = $bindable('sourced'), ...rest }: StatusSelectProps = $props();
</script>

<Select
	{label}
	options={statusOptions}
	bind:value={
		() => value,
		(next) => {
			if (next && isStatus(next)) value = next;
		}
	}
	{...rest}
/>
