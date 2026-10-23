<script lang="ts" module>
	import type { Source } from '#lib/api';
	import type { PostingFilters } from '#lib/postings/api.js';

	export interface PostingFiltersProps {
		filters: PostingFilters;
		oncompany: (company: string) => void;
		onlocation: (location: string) => void;
		onsource: (source: Source | '') => void;
		onremote: (remote: PostingFilters['remote']) => void;
		onincluderemoved: (includeRemoved: boolean) => void;
	}
</script>

<script lang="ts">
	import { Checkbox, Input, Select, type SelectOption } from '#lib/components/ui';
	import { SOURCE_LABELS, SOURCES } from '#lib/postings/api.js';

	let { filters, oncompany, onlocation, onsource, onremote, onincluderemoved }: PostingFiltersProps =
		$props();

	const sourceOptions: SelectOption[] = [
		{ value: '', label: 'Any' },
		...SOURCES.map((source) => ({ value: source, label: SOURCE_LABELS[source] }))
	];
	const remoteOptions: SelectOption[] = [
		{ value: '', label: 'Any' },
		{ value: 'remote', label: 'Remote only' },
		{ value: 'onsite', label: 'Not remote' }
	];
</script>

<div class="flex flex-col gap-4 sm:flex-row sm:flex-wrap sm:items-end">
	<div class="sm:min-w-48 sm:flex-1">
		<Input
			type="search"
			label="Company"
			value={filters.company}
			oninput={(event) => oncompany(event.currentTarget.value)}
		/>
	</div>
	<div class="sm:min-w-48 sm:flex-1">
		<Input
			type="search"
			label="Location"
			value={filters.location}
			oninput={(event) => onlocation(event.currentTarget.value)}
		/>
	</div>
	<div class="sm:w-40">
		<Select
			label="Source"
			options={sourceOptions}
			value={filters.source}
			onchange={(event) => onsource(event.currentTarget.value as Source | '')}
		/>
	</div>
	<div class="sm:w-40">
		<Select
			label="Remote"
			options={remoteOptions}
			value={filters.remote}
			onchange={(event) => onremote(event.currentTarget.value as PostingFilters['remote'])}
		/>
	</div>
	<div class="flex h-10 items-center">
		<Checkbox
			label="Show removed"
			checked={filters.includeRemoved}
			onchange={(event) => onincluderemoved(event.currentTarget.checked)}
		/>
	</div>
</div>
