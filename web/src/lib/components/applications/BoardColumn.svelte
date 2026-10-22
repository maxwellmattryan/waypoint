<script lang="ts">
	import type { TransitionConfig } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import type { Application, ApplicationStatus } from '#lib/api';
	import { boardMotion } from '#lib/applications/board.js';
	import { statusLabel } from '#lib/applications/statuses.js';
	import { Badge } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';
	import BoardCard from './BoardCard.svelte';

	type Crossfade = (node: Element, params: { key: number }) => () => TransitionConfig;

	let {
		status,
		applications,
		pending,
		dragging,
		send,
		receive,
		onmove,
		onpick,
		ondragstart,
		ondragend
	}: {
		status: ApplicationStatus;
		applications: Application[];
		pending: ReadonlySet<number>;
		/** The card being dragged, and the column it came from. */
		dragging: { id: number; from: ApplicationStatus } | null;
		send: Crossfade;
		receive: Crossfade;
		/** A card was dropped here from another column. */
		onmove: (id: number, status: ApplicationStatus) => void;
		/** A status was picked from a card's Move menu. */
		onpick: (id: number, status: ApplicationStatus) => void;
		ondragstart: (application: Application, event: DragEvent) => void;
		ondragend: () => void;
	} = $props();

	const id = $props.id();
	const label = $derived(statusLabel[status]);
	/** Whether the dragged card may drop here: it comes from another column. */
	const accepts = $derived(dragging !== null && dragging.from !== status);
	let over = $state(false);
	const highlighted = $derived(over && accepts);

	function ondragover(event: DragEvent) {
		if (!accepts) return;
		event.preventDefault();
		if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
		over = true;
	}

	function ondragleave(event: DragEvent) {
		const zone = event.currentTarget as HTMLElement;
		if (!zone.contains(event.relatedTarget as Node | null)) over = false;
	}

	function ondrop(event: DragEvent) {
		event.preventDefault();
		over = false;
		if (dragging && dragging.from !== status) onmove(dragging.id, status);
	}
</script>

<section aria-labelledby={id} data-status={status} class="flex w-64 shrink-0 flex-col gap-2 lg:w-auto">
	<div class="flex items-center justify-between gap-2 px-1">
		<h2 {id} class="text-sm font-semibold text-fg">{label}</h2>
		<Badge>{applications.length}</Badge>
	</div>
	<!-- A drop target for the dragged card; the Move menu is the keyboard and touch path. -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		data-drop-zone={status}
		data-over={highlighted || undefined}
		class={[
			'flex min-h-32 flex-1 flex-col gap-2 rounded-lg border p-2 transition-colors',
			highlighted ? 'border-primary bg-primary-soft' : 'border-transparent bg-surface-raised'
		]}
		{ondragover}
		{ondragleave}
		{ondrop}
	>
		<ul aria-label={label} class="flex flex-col gap-2">
			{#each applications as application (application.id)}
				<li
					animate:flip={boardMotion()}
					in:receive={{ key: application.id }}
					out:send={{ key: application.id }}
				>
					<BoardCard
						{application}
						pending={pending.has(application.id)}
						onmove={(to) => onpick(application.id, to)}
						ondragstart={(event) => ondragstart(application, event)}
						{ondragend}
					/>
				</li>
			{/each}
		</ul>
		{#if applications.length === 0}
			<p
				class="rounded-md border border-dashed border-border p-4 text-sm text-fg-muted"
				in:fade={{ duration: motionDuration('base') }}
			>
				No applications
			</p>
		{/if}
	</div>
</section>
