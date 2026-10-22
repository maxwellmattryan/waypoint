<script lang="ts">
	import { tick } from 'svelte';
	import { crossfade, fade } from 'svelte/transition';
	import type { Application, ApplicationStatus } from '#lib/api';
	import type { BoardColumnData } from '#lib/applications/applications.svelte.js';
	import { boardMotion } from '#lib/applications/board.js';
	import { easings, motionDuration } from '#lib/motion.js';
	import BoardColumn from './BoardColumn.svelte';

	let {
		columns,
		pending,
		onmove
	}: {
		/** The five columns in board order. */
		columns: BoardColumnData[];
		/** Ids whose move is in flight. */
		pending: ReadonlySet<number>;
		/**
		 * Moves a card. When it returns a promise (the store's `move`), a Move-menu pick waits
		 * for it and then focuses the card's Move button wherever the card ended up.
		 */
		onmove: (id: number, status: ApplicationStatus) => unknown;
	} = $props();

	const [send, receive] = crossfade({
		duration: () => boardMotion().duration,
		easing: easings.standard,
		fallback: (node) => fade(node, { duration: motionDuration('fast') })
	});

	let root = $state<HTMLElement>();
	// Held here rather than read from DataTransfer, which jsdom lacks.
	let dragging = $state<{ id: number; from: ApplicationStatus } | null>(null);

	function ondragstart(application: Application, event: DragEvent) {
		dragging = { id: application.id, from: application.status };
		if (event.dataTransfer) {
			event.dataTransfer.effectAllowed = 'move';
			event.dataTransfer.setData('text/plain', String(application.id));
		}
	}

	function ondragend() {
		dragging = null;
	}

	function drop(id: number, status: ApplicationStatus) {
		dragging = null;
		onmove(id, status);
	}

	/** Focuses the card's Move button in the column that holds it now. */
	function focusTrigger(id: number) {
		const status = columns.find((c) => c.applications.some((a) => a.id === id))?.status;
		if (!status || !root) return;
		const triggers = root.querySelectorAll<HTMLElement>(
			`[data-status="${status}"] [data-move-trigger="${id}"]`
		);
		// Skip a copy that's still fading out of its old column (Svelte marks it inert).
		[...triggers].find((trigger) => !trigger.closest('[inert]'))?.focus();
	}

	async function pick(id: number, status: ApplicationStatus) {
		await onmove(id, status);
		await tick();
		focusTrigger(id);
	}
</script>

<div
	bind:this={root}
	class="relative flex gap-4 overflow-x-auto pb-4 lg:grid lg:grid-cols-5 lg:overflow-visible"
>
	{#each columns as column (column.status)}
		<BoardColumn
			status={column.status}
			applications={column.applications}
			{pending}
			{dragging}
			{send}
			{receive}
			onmove={drop}
			onpick={pick}
			{ondragstart}
			{ondragend}
		/>
	{/each}
</div>
