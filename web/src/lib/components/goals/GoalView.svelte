<script lang="ts" module>
	import type { GoalDetail, PlanEdit } from '#lib/api';
	import type { Answer, GoalResult } from '#lib/goals/api.js';
	import type { GoalState } from '#lib/goals/goal.svelte.js';

	export interface GoalViewProps {
		view: GoalState;
		/** An action is in flight; its controls are disabled. */
		busy?: boolean;
		onanswer: (answers: Answer[]) => void;
		onrun: () => void;
		/** Asks for a replan; resolves `true` once it was queued. */
		onreplan: (instruction: string) => Promise<boolean>;
		onsave: (body: PlanEdit) => Promise<GoalResult<GoalDetail>>;
		/** Loads the goal again after a load error. */
		onreload: () => void;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { fade } from 'svelte/transition';
	import RunLink from '#lib/components/review/RunLink.svelte';
	import { Button, Card, EmptyState, Link, Skeleton } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';
	import ClarifyingQuestionsForm from './ClarifyingQuestionsForm.svelte';
	import GoalStatusBadge from './GoalStatusBadge.svelte';
	import PlanEditor from './PlanEditor.svelte';
	import PlanView from './PlanView.svelte';
	import ReplanDialog from './ReplanDialog.svelte';

	/** One goal's page body: every state of a goal, its plan and its controls. */
	let { view, busy = false, onanswer, onrun, onreplan, onsave, onreload }: GoalViewProps = $props();

	const listHref = resolve('/goals');
	let editing = $state(false);
	let replanning = $state(false);

	const goal = $derived(view.kind === 'loaded' ? view.goal : null);
	const open = $derived(goal?.questions.filter((q) => q.answered_at === null) ?? []);
	/** Which status block shows; a change fades between them. */
	const block = $derived(goal?.status ?? view.kind);
</script>

{#if view.kind === 'loading'}
	<div class="flex flex-col gap-4" aria-busy="true" aria-label="Loading goal">
		<Skeleton class="h-8 w-64" />
		<Skeleton class="h-32" />
		<Skeleton class="h-48" />
	</div>
{:else if view.kind === 'not-found'}
	<div in:fade={{ duration: motionDuration('base') }}>
		<EmptyState title="No such goal" description="The link may be wrong, or the goal was deleted.">
			{#snippet action()}
				<Link href={listHref}>All goals</Link>
			{/snippet}
		</EmptyState>
	</div>
{:else if view.kind === 'error'}
	<div in:fade={{ duration: motionDuration('base') }}>
		<EmptyState
			title="Couldn't load this goal"
			description="Check that the API is running, then try again."
		>
			{#snippet action()}
				<Button variant="secondary" onclick={onreload}>Try again</Button>
			{/snippet}
		</EmptyState>
	</div>
{:else if goal}
	<div class="flex flex-col gap-8" in:fade={{ duration: motionDuration('base') }}>
		<div class="flex flex-col gap-2">
			<div class="flex flex-wrap items-center gap-2">
				<GoalStatusBadge status={goal.status} />
				{#if goal.versions.length > 0}
					<span class="text-xs text-fg-muted">
						{goal.versions.length === 1 ? '1 version' : `${goal.versions.length} versions`}
					</span>
				{/if}
			</div>
			<h1 class="text-2xl font-semibold break-words text-fg">{goal.text}</h1>
		</div>

		{#key block}
			<div in:fade={{ duration: motionDuration('base') }}>
				{#if goal.status === 'planning'}
					<Card class="flex flex-col gap-4" aria-busy="true">
						<div class="flex flex-wrap items-center gap-x-4 gap-y-1">
							<p class="text-sm font-semibold text-fg" role="status">
								{goal.run?.status === 'paused' ? 'Paused: agents are paused' : 'Planning…'}
							</p>
							{#if goal.run}
								<RunLink runId={goal.run.id} />
							{/if}
						</div>
						<div class="flex flex-col gap-2" aria-hidden="true">
							<Skeleton class="h-4" />
							<Skeleton class="h-4 w-2/3" />
							<Skeleton class="h-16" />
						</div>
					</Card>
				{:else if goal.status === 'needs_answers'}
					<ClarifyingQuestionsForm questions={open} {busy} onsubmit={onanswer} />
				{:else if goal.status === 'failed'}
					<Card class="flex flex-col items-start gap-2">
						<p class="text-sm font-semibold text-danger" role="alert">This goal failed</p>
						{#if goal.error}
							<p class="text-sm break-words whitespace-pre-wrap text-fg-muted">{goal.error}</p>
						{/if}
						{#if goal.plan?.status === 'ready'}
							<Button size="sm" disabled={busy} onclick={onrun}>Run again</Button>
						{:else}
							<Button size="sm" variant="secondary" disabled={busy} onclick={() => (replanning = true)}>
								Replan
							</Button>
						{/if}
					</Card>
				{:else if goal.status === 'running'}
					<p class="text-sm text-fg-muted" role="status">
						Running the plan. Steps update as they finish.
					</p>
				{:else if goal.status === 'completed'}
					<p class="text-sm text-fg-muted" role="status">
						The plan finished. Edit it or replan to run it again.
					</p>
				{/if}
			</div>
		{/key}

		{#if goal.plan}
			<PlanView
				plan={goal.plan}
				status={goal.status}
				{busy}
				{onrun}
				onedit={() => (editing = true)}
				onreplan={() => (replanning = true)}
			/>
			<PlanEditor bind:open={editing} plan={goal.plan} {onsave} />
		{/if}
		<ReplanDialog bind:open={replanning} {busy} onsubmit={onreplan} />
	</div>
{/if}
