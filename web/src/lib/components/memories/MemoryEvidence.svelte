<script lang="ts" module>
	import type { MemorySignal } from '#lib/api';
	import type { Result } from '#lib/memories/api.js';

	export interface MemoryEvidenceProps {
		memoryId: number;
		/** Reads the memory's evidence signals; called on first expand and on "Try again". */
		load: (id: number) => Promise<Result<MemorySignal[]>>;
	}
</script>

<script lang="ts">
	import { fade, slide } from 'svelte/transition';
	import SignalDetails from '#lib/components/review/SignalDetails.svelte';
	import { Button, Skeleton } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';

	let { memoryId, load }: MemoryEvidenceProps = $props();

	const panelId = $props.id();

	let expanded = $state(false);
	let view = $state<
		| { kind: 'idle' }
		| { kind: 'loading' }
		| { kind: 'error' }
		| { kind: 'loaded'; signals: MemorySignal[] }
	>({ kind: 'idle' });

	function toggle() {
		expanded = !expanded;
		if (expanded && view.kind === 'idle') void fetch();
	}

	async function fetch() {
		if (view.kind === 'loading') return;
		view = { kind: 'loading' };
		const result = await load(memoryId);
		view = result.kind === 'ok' ? { kind: 'loaded', signals: result.data } : { kind: 'error' };
	}
</script>

<div class="flex flex-col gap-2">
	<div>
		<Button
			variant="ghost"
			size="sm"
			aria-expanded={expanded}
			aria-controls={panelId}
			onclick={toggle}
		>
			{expanded ? 'Hide evidence' : 'Show evidence'}
		</Button>
	</div>

	{#if expanded}
		<div
			id={panelId}
			transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}
		>
			{#if view.kind === 'loading' || view.kind === 'idle'}
				<div class="flex flex-col gap-2" aria-busy="true" aria-label="Loading evidence">
					<Skeleton class="h-16" />
					<Skeleton class="h-16" />
				</div>
			{:else if view.kind === 'error'}
				<div
					class="flex flex-wrap items-center gap-2"
					in:fade={{ duration: motionDuration('fast') }}
				>
					<p class="text-sm text-fg-muted">Couldn't load the evidence.</p>
					<Button variant="secondary" size="sm" onclick={fetch}>Try again</Button>
				</div>
			{:else if view.signals.length === 0}
				<p class="text-sm text-fg-muted" in:fade={{ duration: motionDuration('fast') }}>
					No decisions are linked to this memory any more.
				</p>
			{:else}
				<ul
					aria-label="Evidence"
					class="flex flex-col divide-y divide-border rounded-md border border-border"
					in:fade={{ duration: motionDuration('fast') }}
				>
					{#each view.signals as signal (signal.id)}
						<li class="p-4">
							<SignalDetails {signal} />
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	{/if}
</div>
