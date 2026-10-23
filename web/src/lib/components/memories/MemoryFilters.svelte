<script lang="ts" module>
	import type { MemoryStatus } from '#lib/api';

	export interface MemoryFiltersProps {
		q: string;
		status: MemoryStatus | '';
		/** The active tag filter; blank for none. */
		tag: string;
		onquery: (q: string) => void;
		onstatus: (status: MemoryStatus | '') => void;
		/** Clears the tag filter. */
		oncleartag: () => void;
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { Input, Select, Tag } from '#lib/components/ui';
	import { STATUS_OPTIONS } from '#lib/memories/topics.js';
	import { motionDuration } from '#lib/motion.js';

	let { q, status, tag, onquery, onstatus, oncleartag }: MemoryFiltersProps = $props();
</script>

<div class="flex flex-col gap-4 sm:flex-row sm:items-end">
	<div class="flex-1">
		<Input
			type="search"
			label="Search memories"
			placeholder="e.g. remote, salary"
			value={q}
			oninput={(event) => onquery(event.currentTarget.value)}
		/>
	</div>
	<div class="sm:w-48">
		<Select
			label="Status"
			options={STATUS_OPTIONS}
			value={status}
			onchange={(event) => onstatus(event.currentTarget.value as MemoryStatus | '')}
		/>
	</div>
	{#if tag}
		<div class="flex h-10 items-center gap-2" transition:fade={{ duration: motionDuration('fast') }}>
			<span class="text-sm text-fg-muted">Tag</span>
			<Tag label={tag} onremove={oncleartag} />
		</div>
	{/if}
</div>
