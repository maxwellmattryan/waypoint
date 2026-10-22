<script lang="ts" module>
	import type { Stage } from '#lib/api';
	import type { StageRowProps } from './StageRow.svelte';

	export interface StageListProps extends Pick<StageRowProps, 'ontoggle' | 'onmove' | 'onremove'> {
		/** In order. */
		stages: Stage[];
		/** A stage write is in flight: every control is disabled. */
		busy?: boolean;
		/** Adds a stage with this (trimmed, non-blank) name; resolves `true` on success, which clears the input. */
		onadd: (name: string) => Promise<boolean>;
	}
</script>

<script lang="ts">
	import { tick } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import { Button, Input } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import StageRow from './StageRow.svelte';

	let { stages, busy = false, onadd, ontoggle, onmove, onremove }: StageListProps = $props();

	const inputId = $props.id();
	let name = $state('');
	let error = $state<string>();

	async function add(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		const trimmed = name.trim();
		if (!trimmed) {
			error = 'Required';
			return;
		}
		error = undefined;
		if (await onadd(trimmed)) {
			name = '';
			// The input was disabled while the stage was sent; put the cursor back for the next one.
			await tick();
			document.getElementById(inputId)?.focus();
		}
	}
</script>

<div class="flex flex-col gap-4">
	{#if stages.length === 0}
		<p class="text-sm text-fg-muted" in:fade={{ duration: motionDuration('fast') }}>
			No stages yet
		</p>
	{:else}
		<ol class="flex flex-col gap-2" aria-label="Stages">
			{#each stages as stage, index (stage.id)}
				<li
					animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
					transition:fade={{ duration: motionDuration('fast') }}
				>
					<StageRow
						{stage}
						first={index === 0}
						last={index === stages.length - 1}
						disabled={busy}
						{ontoggle}
						{onmove}
						{onremove}
					/>
				</li>
			{/each}
		</ol>
	{/if}

	<form class="flex items-start gap-2" novalidate onsubmit={add}>
		<div class="min-w-0 flex-1">
			<Input
				id={inputId}
				label="New stage"
				bind:value={name}
				{error}
				maxlength={200}
				autocomplete="off"
				disabled={busy}
				oninput={() => (error = undefined)}
			/>
		</div>
		<Button type="submit" variant="secondary" class="mt-6" disabled={busy}>Add stage</Button>
	</form>
</div>
