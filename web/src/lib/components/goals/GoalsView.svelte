<script lang="ts" module>
	import type { Goal } from '#lib/api';
	import type { GoalsView } from '#lib/goals/goals.svelte.js';

	export interface GoalsViewProps {
		view: GoalsView;
		goals: Goal[];
		/** A goal is being created. */
		creating?: boolean;
		/** The composer's error. */
		error?: string | null;
		oncreate: (text: string) => Promise<boolean>;
		/** Loads the list again after a load error. */
		onreload: () => void;
	}
</script>

<script lang="ts">
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { Button, Card, EmptyState, Skeleton } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import GoalComposer from './GoalComposer.svelte';
	import GoalListItem from './GoalListItem.svelte';

	/** The Goals page body: the composer and the list with its loading, empty and error states. */
	let { view, goals, creating = false, error = null, oncreate, onreload }: GoalsViewProps =
		$props();
</script>

<div class="flex flex-col gap-8">
	<div class="flex flex-col gap-1">
		<h1 class="text-2xl font-semibold text-fg">Goals</h1>
		<p class="text-sm text-fg-muted">
			Say what you're after. Waypoint plans which companies to source, scores what it finds and
			shortlists the best fits.
		</p>
	</div>

	<Card>
		<GoalComposer busy={creating} {error} onsubmit={oncreate} />
	</Card>

	{#if view.kind === 'loading'}
		<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading goals">
			{#each [0, 1, 2] as row (row)}
				<Skeleton class="h-16" />
			{/each}
		</div>
	{:else if view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load your goals"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={onreload}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if goals.length === 0}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState title="No goals yet" description="Describe what you're looking for above." />
		</div>
	{:else}
		<ul class="flex flex-col gap-2" aria-label="Goals">
			{#each goals as goal (goal.id)}
				<li
					animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
					in:fade={{ duration: motionDuration('base') }}
				>
					<GoalListItem {goal} />
				</li>
			{/each}
		</ul>
	{/if}
</div>
