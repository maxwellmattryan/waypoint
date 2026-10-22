<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { TooltipPlacement } from './styles';

	/** Spread onto the trigger so it points at the tooltip while it is visible. */
	export type TooltipTriggerProps = { 'aria-describedby': string | undefined };

	export interface TooltipProps {
		content: string;
		/** Default `top`. */
		placement?: TooltipPlacement;
		/** Hover delay in ms. Default `300`. Keyboard focus shows it at once. */
		delay?: number;
		/** Renders the trigger. Spread the argument onto it. */
		children: Snippet<[TooltipTriggerProps]>;
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
	import { layers, tooltipPlacements } from './styles';

	let { content, placement = 'top', delay = 300, children }: TooltipProps = $props();

	const id = $props.id();
	let visible = $state(false);
	let timer: ReturnType<typeof setTimeout> | undefined;

	function cancel() {
		clearTimeout(timer);
		timer = undefined;
	}

	function showLater() {
		cancel();
		timer = setTimeout(() => (visible = true), delay);
	}

	function show() {
		cancel();
		visible = true;
	}

	function hide() {
		cancel();
		visible = false;
	}

	function onkeydown(event: KeyboardEvent) {
		if (event.key !== 'Escape' || !visible) return;
		// Keep Escape from also closing a Dialog or Drawer behind the tooltip.
		event.stopPropagation();
		hide();
	}

	$effect(() => cancel);
</script>

<!-- The wrapper only listens for events bubbling from the trigger; the trigger is the control. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<span
	class="relative inline-flex"
	onpointerenter={showLater}
	onpointerleave={hide}
	onfocusin={show}
	onfocusout={hide}
	{onkeydown}
>
	{@render children({ 'aria-describedby': visible ? id : undefined })}
	{#if visible}
		<span
			role="tooltip"
			{id}
			class={[
				'pointer-events-none absolute rounded-sm bg-fg px-2 py-1 text-xs whitespace-nowrap text-bg',
				layers.tooltip,
				tooltipPlacements[placement]
			]}
			transition:fade={{ duration: motionDuration('fast'), easing: easings.standard }}
		>
			{content}
		</span>
	{/if}
</span>
