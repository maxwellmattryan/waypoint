<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createApiClient } from '#lib/api';
	import GoalView from '#lib/components/goals/GoalView.svelte';
	import { Link } from '#lib/components/ui';
	import { getDashboard } from '#lib/dashboard/dashboard.svelte.js';
	import { GoalStore, type GoalState } from '#lib/goals/goal.svelte.js';

	const client = createApiClient();
	const dashboard = getDashboard();
	const listHref = resolve('/goals');

	/** The route's id as a positive integer, or `null` for anything else (no request is made). */
	const id = $derived.by(() => {
		const raw = page.params.id ?? '';
		const parsed = /^[1-9]\d*$/.test(raw) ? Number(raw) : null;
		return parsed !== null && Number.isSafeInteger(parsed) ? parsed : null;
	});

	let store = $state<GoalStore | null>(null);

	$effect(() => {
		if (id === null) {
			store = null;
			return;
		}
		const next = new GoalStore(id, { client });
		store = next;
		void next.load();
		const unsubscribe = dashboard.subscribe((event) => next.handle(event));
		return () => {
			unsubscribe();
			next.destroy();
		};
	});

	const view: GoalState = $derived(store?.state ?? { kind: 'not-found' });

	async function replan(instruction: string): Promise<boolean> {
		const result = await store?.replan(instruction);
		return result?.kind === 'ok';
	}
</script>

<svelte:head>
	<title>{view.kind === 'loaded' ? view.goal.text.slice(0, 60) : 'Goal'} · Waypoint</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<Link href={listHref} class="self-start text-sm">Back to goals</Link>
	<GoalView
		{view}
		busy={!!store?.busy}
		onanswer={(answers) => store?.answer(answers)}
		onrun={() => store?.run()}
		onreplan={replan}
		onsave={(body) => store?.edit(body) ?? Promise.resolve({ kind: 'failed' })}
		onreload={() => store?.load()}
	/>
</div>
