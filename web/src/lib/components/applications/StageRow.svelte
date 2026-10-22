<script lang="ts" module>
	import type { Stage } from '#lib/api';

	export interface StageRowProps {
		stage: Stage;
		/** Disables "Move up". */
		first: boolean;
		/** Disables "Move down". */
		last: boolean;
		/** Disables every control, e.g. while a stage write is in flight. */
		disabled?: boolean;
		/** Completes or reopens the stage; resolves `false` when that failed, which unticks the box again. */
		ontoggle: (stage: Stage, completed: boolean) => Promise<boolean>;
		onmove: (stage: Stage, delta: -1 | 1) => void;
		onremove: (stage: Stage) => void;
	}
</script>

<script lang="ts">
	import { Checkbox, IconButton } from '#lib/components/ui';
	import CloseIcon from '#lib/components/ui/CloseIcon.svelte';

	let { stage, first, last, disabled = false, ontoggle, onmove, onremove }: StageRowProps =
		$props();

	async function toggle(event: Event & { currentTarget: HTMLInputElement }) {
		const input = event.currentTarget;
		const completed = input.checked;
		// The box shows the server's answer: on failure it goes back to what it was.
		if (!(await ontoggle(stage, completed))) input.checked = !completed;
	}
</script>

{#snippet chevron(path: string)}
	<svg
		class="size-4"
		viewBox="0 0 16 16"
		fill="none"
		stroke="currentColor"
		stroke-width="1.5"
		aria-hidden="true"
	>
		<path d={path} stroke-linecap="round" stroke-linejoin="round" />
	</svg>
{/snippet}

<div class="flex items-center justify-between gap-2">
	<div class="min-w-0 break-words">
		<Checkbox
			label={stage.name}
			checked={stage.completed_at !== null}
			{disabled}
			onchange={toggle}
		/>
	</div>
	<div class="flex shrink-0 gap-1">
		<IconButton
			label="Move up"
			size="sm"
			disabled={disabled || first}
			onclick={() => onmove(stage, -1)}
		>
			{@render chevron('M4 10l4-4 4 4')}
		</IconButton>
		<IconButton
			label="Move down"
			size="sm"
			disabled={disabled || last}
			onclick={() => onmove(stage, 1)}
		>
			{@render chevron('M4 6l4 4 4-4')}
		</IconButton>
		<IconButton label="Remove stage" size="sm" {disabled} onclick={() => onremove(stage)}>
			<CloseIcon />
		</IconButton>
	</div>
</div>
