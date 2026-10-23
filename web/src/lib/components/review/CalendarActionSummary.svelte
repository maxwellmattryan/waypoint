<script lang="ts" module>
	import type { CalendarProposal } from '#lib/review/calendar.js';

	export interface CalendarActionSummaryProps {
		proposal: CalendarProposal;
	}
</script>

<script lang="ts">
	import { KeyValue, type KeyValueItem } from '#lib/components/ui';

	/** A proposed Google Calendar event, readable: its title, when it is and its notes. */
	let { proposal }: CalendarActionSummaryProps = $props();

	const items = $derived.by(() => {
		const rows: KeyValueItem[] = [
			{ key: 'Event', value: proposal.summary },
			{ key: 'When', value: proposal.when },
			{ key: 'Time zone', value: proposal.zone }
		];
		if (proposal.description.length > 0) rows.push({ key: 'Details', value: details });
		return rows;
	});
</script>

{#snippet details()}
	<ul class="flex flex-col gap-1" aria-label="Event details">
		{#each proposal.description as line, i (i)}
			<li>{line}</li>
		{/each}
	</ul>
{/snippet}

<section class="flex flex-col gap-2" aria-label="Add to Google Calendar">
	<h3 class="text-sm font-semibold text-fg">Add to Google Calendar</h3>
	<KeyValue {items} />
</section>
