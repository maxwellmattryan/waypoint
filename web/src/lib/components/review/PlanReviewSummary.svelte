<script lang="ts" module>
	import type { PlanReview } from '#lib/review/plan.js';

	export interface PlanReviewSummaryProps {
		summary: PlanReview;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import CompanyRow from '#lib/components/goals/CompanyRow.svelte';
	import { KeyValue, Link, type KeyValueItem } from '#lib/components/ui';

	/** A plan review item, readable: the goal, the summary, the companies and what was dropped. */
	let { summary }: PlanReviewSummaryProps = $props();

	const goalHref = $derived(resolve('/goals/[id]', { id: String(summary.goalId) }));
	const droppedAny = $derived(
		summary.dropped.companies.length +
			summary.dropped.keywords.length +
			summary.dropped.locations.length >
			0
	);

	const items = $derived<KeyValueItem[]>([
		{ key: 'Goal', value: goal },
		{ key: 'Plan', value: `Version ${summary.version}: ${summary.summary}` },
		{ key: 'Companies', value: companies },
		...(droppedAny ? [{ key: 'Dropped', value: dropped }] : [])
	]);
</script>

{#snippet goal()}
	<Link href={goalHref}>{summary.goal}</Link>
{/snippet}

{#snippet companies()}
	<ul class="flex flex-col divide-y divide-border">
		{#each summary.companies as company (`${company.source}/${company.board}`)}
			<li><CompanyRow {company} /></li>
		{/each}
	</ul>
{/snippet}

{#snippet dropped()}
	<ul class="flex flex-col gap-1">
		{#each summary.dropped.companies as company, index (index)}
			<li class="break-words">
				{company.name} <span class="font-mono text-xs text-fg-muted">{company.source}/{company.board}</span>:
				{company.why}
			</li>
		{/each}
		{#if summary.dropped.keywords.length > 0}
			<li class="break-words">Keywords: {summary.dropped.keywords.join(', ')}</li>
		{/if}
		{#if summary.dropped.locations.length > 0}
			<li class="break-words">Locations: {summary.dropped.locations.join(', ')}</li>
		{/if}
	</ul>
{/snippet}

<KeyValue {items} />
