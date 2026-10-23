<script lang="ts" module>
	import type { PostingPacketsState } from '#lib/packets/packets.svelte.js';

	export interface PostingPacketsPanelProps {
		state: PostingPacketsState;
		/** A prepare request is in flight; the prepare buttons are disabled. */
		starting?: boolean;
		/** Queues a packet: Prepare packet, Prepare again and Try again after a failed run. */
		onprepare: () => void;
		/** Loads the packets again after a load error. */
		onreload: () => void;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { fade, slide } from 'svelte/transition';
	import type { FitRun } from '#lib/api';
	import { formatDateTime } from '#lib/applications/format.js';
	import RunLink from '#lib/components/review/RunLink.svelte';
	import { Button, Card, Link, Skeleton } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import { answeredCount, QUESTION_COUNT } from '#lib/packets/api.js';
	import PacketStatusBadge from './PacketStatusBadge.svelte';

	/** The posting page's Packets section: every state of a posting's application packets. */
	let { state, starting = false, onprepare, onreload }: PostingPacketsPanelProps = $props();

	const view = $derived(state.kind === 'loaded' ? state.view : null);
	const scored = $derived(state.kind === 'loaded' && state.packets.scored);
	/** Which block shows; a change fades between them. */
	const shown = $derived(view?.kind ?? state.kind);
	const busy = $derived(
		view?.kind === 'preparing' ||
			view?.kind === 'paused' ||
			(view?.kind === 'list' && view.preparing !== null)
	);
	const headerLabel = $derived.by(() => {
		if (view?.kind === 'preparing' || view?.kind === 'paused') return 'Preparing…';
		if (view?.kind === 'failed') return 'Try again';
		return 'Prepare packet';
	});
	const packetHref = (id: number) => resolve('/packets/[id]', { id: String(id) });
</script>

{#snippet preparing(run: FitRun)}
	<Card class="flex flex-col gap-4" aria-busy="true">
		<div class="flex flex-wrap items-center gap-x-4 gap-y-1">
			<p class="text-sm font-semibold text-fg" role="status">
				{run.status === 'paused' ? 'Paused: agents are paused' : 'Preparing…'}
			</p>
			<RunLink runId={run.id} />
		</div>
		<div class="flex flex-col gap-2" aria-hidden="true">
			<Skeleton class="h-4 w-1/2" />
			<Skeleton class="h-4" />
			<Skeleton class="h-4 w-2/3" />
		</div>
	</Card>
{/snippet}

{#snippet failed(run: FitRun)}
	<Card class="flex flex-col gap-2">
		<div class="flex flex-wrap items-center gap-x-4 gap-y-1">
			<p class="text-sm font-semibold text-danger" role="alert">Preparing failed</p>
			<RunLink runId={run.id} />
		</div>
		{#if run.error}
			<p class="text-sm break-words whitespace-pre-wrap text-fg-muted">{run.error}</p>
		{/if}
	</Card>
{/snippet}

<section class="flex flex-col gap-4" aria-labelledby="posting-packets-heading">
	<div class="flex flex-wrap items-center justify-between gap-4">
		<h2 id="posting-packets-heading" class="text-lg font-semibold text-fg">Packets</h2>
		{#if state.kind === 'loaded' && view?.kind !== 'list'}
			<Button
				variant={view?.kind === 'empty' ? 'primary' : 'secondary'}
				size="sm"
				disabled={starting || busy || !scored}
				onclick={onprepare}
			>
				{headerLabel}
			</Button>
		{/if}
	</div>

	{#key shown}
		<div in:fade={{ duration: motionDuration('base') }}>
			{#if state.kind === 'loading'}
				<div aria-busy="true" aria-label="Loading packets">
					<Skeleton class="h-24" />
				</div>
			{:else if state.kind === 'error'}
				<Card class="flex flex-col items-start gap-2">
					<p class="text-sm font-semibold text-fg">Couldn't load packets</p>
					<p class="text-sm text-fg-muted">Check that the API is running, then try again.</p>
					<Button variant="secondary" size="sm" onclick={onreload}>Try again</Button>
				</Card>
			{:else if view?.kind === 'notScored'}
				<Card class="flex flex-col gap-1">
					<p class="text-sm font-semibold text-fg">No packet yet</p>
					<p class="text-sm text-fg-muted">Score this posting's fit before preparing a packet.</p>
				</Card>
			{:else if view?.kind === 'empty'}
				<Card class="flex flex-col gap-1">
					<p class="text-sm font-semibold text-fg">No packet yet</p>
					<p class="text-sm text-fg-muted">
						Waypoint picks a resume and drafts answers to common application questions from what
						it knows about you.
					</p>
				</Card>
			{:else if view?.kind === 'preparing' || view?.kind === 'paused'}
				{@render preparing(view.run)}
			{:else if view?.kind === 'failed'}
				{@render failed(view.run)}
			{:else if view?.kind === 'list'}
				<div class="flex flex-col gap-4">
					{#if view.preparing}
						<div transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}>
							{@render preparing(view.preparing)}
						</div>
					{:else if view.failed}
						<div transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}>
							{@render failed(view.failed)}
						</div>
					{/if}
					<ul class="flex flex-col gap-2" aria-label="Packets">
						{#each view.packets as packet (packet.id)}
							<li in:fade={{ duration: motionDuration('base') }}>
								<Card class="flex flex-wrap items-center justify-between gap-4">
									<div class="flex min-w-0 flex-col gap-1">
										<div class="flex flex-wrap items-center gap-2">
											<PacketStatusBadge status={packet.status} />
											<span class="text-sm font-semibold break-words text-fg">
												{packet.resume_name}
											</span>
										</div>
										<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted tabular-nums">
											<span>{answeredCount(packet)} of {QUESTION_COUNT} answered</span>
											<span>Prepared {formatDateTime(packet.created_at)}</span>
										</p>
									</div>
									<Link href={packetHref(packet.id)} class="text-sm">View packet</Link>
								</Card>
							</li>
						{/each}
					</ul>
					<Button
						variant="secondary"
						size="sm"
						class="self-start"
						disabled={starting || busy || !scored}
						onclick={onprepare}
					>
						Prepare again
					</Button>
				</div>
			{/if}
		</div>
	{/key}
</section>
