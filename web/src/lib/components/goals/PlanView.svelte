<script lang="ts" module>
	import type { GoalStatus, Plan } from '#lib/api';

	export interface PlanViewProps {
		plan: Plan;
		/** The goal's status, which decides which controls are enabled. */
		status: GoalStatus;
		/** An action is in flight; the controls are disabled. */
		busy?: boolean;
		onrun: () => void;
		onedit: () => void;
		onreplan: () => void;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { slide } from 'svelte/transition';
	import { Button, Card, KeyValue, Link, type KeyValueItem } from '#lib/components/ui';
	import { versionLine } from '#lib/goals/format.js';
	import { easings, motionDuration } from '#lib/motion.js';
	import { confidenceLabel } from '#lib/postings/fit.js';
	import { formatCost } from '#lib/runs/format.js';
	import CompanyRow from './CompanyRow.svelte';
	import PlanPacketList from './PlanPacketList.svelte';
	import PlanStepList from './PlanStepList.svelte';
	import ShortlistList from './ShortlistList.svelte';

	/** A goal's current plan: what it targets, its steps, its shortlist and its packets, with Run, Edit and Replan. */
	let { plan, status, busy = false, onrun, onedit, onreplan }: PlanViewProps = $props();

	const reviewHref = resolve('/review');
	const canRun = $derived((status === 'planned' || status === 'failed') && plan.status === 'ready');
	const canEdit = $derived(status !== 'planning' && status !== 'needs_answers');
	const canReplan = $derived(status === 'planned' || status === 'completed' || status === 'failed');
	const meta = $derived(
		[
			versionLine(plan),
			plan.confidence === null ? null : confidenceLabel(plan.confidence),
			plan.author === 'planner' ? formatCost(plan.cost_usd) : null
		].filter((part) => part !== null)
	);
	const filters = $derived<KeyValueItem[]>([
		{ key: 'Title keywords', value: plan.title_keywords.join(', ') },
		{ key: 'Locations', value: plan.locations.length ? plan.locations.join(', ') : 'Anywhere' },
		{ key: 'Remote only', value: plan.remote_only ? 'Yes' : 'No' },
		{ key: 'Most to score', value: String(plan.max_to_score) }
	]);
	const shortlistSettings = $derived<KeyValueItem[]>([
		{ key: 'Size', value: String(plan.shortlist_size) },
		{ key: 'Minimum score', value: String(plan.min_score) }
	]);
</script>

<section class="flex flex-col gap-4" aria-labelledby="plan-heading">
	<div class="flex flex-wrap items-start justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<h2 id="plan-heading" class="text-lg font-semibold text-fg">Plan</h2>
			<p class="text-sm break-words text-fg">{plan.summary}</p>
			<p class="text-xs text-fg-muted tabular-nums">{meta.join(' · ')}</p>
		</div>
		<div class="flex flex-wrap gap-2">
			<Button variant="secondary" disabled={busy || !canReplan} onclick={onreplan}>Replan</Button>
			<Button variant="secondary" disabled={busy || !canEdit} onclick={onedit}>Edit plan</Button>
			<Button disabled={busy || !canRun} onclick={onrun}>
				{status === 'failed' ? 'Run again' : 'Run'}
			</Button>
		</div>
	</div>

	{#if plan.status === 'in_review'}
		<p
			class="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-md bg-warning-soft px-4 py-2 text-sm text-warning-soft-fg"
			transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}
		>
			<span>This plan needs review before it can run.</span>
			<Link href={reviewHref}>Open the review queue</Link>
		</p>
	{:else if plan.status === 'dismissed'}
		<p
			class="rounded-md bg-neutral-soft px-4 py-2 text-sm text-neutral-soft-fg"
			transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}
		>
			You dismissed this plan. Edit it or replan.
		</p>
	{/if}

	<Card class="flex flex-col gap-2">
		<h3 class="text-sm font-semibold text-fg">Target companies</h3>
		<ul class="flex flex-col divide-y divide-border" aria-label="Target companies">
			{#each plan.companies as company (`${company.source}/${company.board}`)}
				<li><CompanyRow {company} /></li>
			{/each}
		</ul>
	</Card>

	<div class="grid gap-4 sm:grid-cols-2">
		<Card class="flex flex-col gap-2">
			<h3 class="text-sm font-semibold text-fg">Filters</h3>
			<KeyValue items={filters} />
		</Card>
		<Card class="flex flex-col gap-2">
			<h3 class="text-sm font-semibold text-fg">Shortlist settings</h3>
			<KeyValue items={shortlistSettings} />
		</Card>
	</div>

	{#if plan.prep_notes}
		<Card class="flex flex-col gap-2">
			<h3 class="text-sm font-semibold text-fg">Prep notes</h3>
			<p class="text-sm break-words whitespace-pre-wrap text-fg">{plan.prep_notes}</p>
		</Card>
	{/if}

	<Card class="flex flex-col gap-2">
		<h3 class="text-sm font-semibold text-fg">Steps</h3>
		<PlanStepList steps={plan.steps} />
	</Card>

	{#if plan.shortlist.length > 0}
		<Card class="flex flex-col gap-2">
			<h3 class="text-sm font-semibold text-fg">Shortlist</h3>
			<ShortlistList entries={plan.shortlist} />
		</Card>
	{/if}

	{#if plan.packets.length > 0}
		<Card class="flex flex-col gap-2">
			<h3 class="text-sm font-semibold text-fg">Packets</h3>
			<PlanPacketList packets={plan.packets} />
		</Card>
	{/if}
</section>
