<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { DrawerSide } from './styles';

	export interface DrawerProps {
		/** Whether the drawer is shown. Bindable. */
		open?: boolean;
		/** Heading, and the drawer's accessible name. */
		title: string;
		/** Optional text under the title, linked with `aria-describedby`. */
		description?: string;
		/** The edge it slides in from. Default `right`. */
		side?: DrawerSide;
		/** Close when the backdrop is clicked. Default `true`. */
		closeOnBackdrop?: boolean;
		/** Called after the drawer closes itself (Escape, Close button or backdrop). */
		onclose?: () => void;
		children?: Snippet;
		/** Actions row pinned to the bottom. */
		footer?: Snippet;
	}
</script>

<script lang="ts">
	import { fly } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
	import Backdrop from './Backdrop.svelte';
	import OverlayHeader from './OverlayHeader.svelte';
	import { trapFocus } from './focus-trap';
	import { pushOverlay } from './overlay';
	import { layers } from './styles';

	let {
		open = $bindable(false),
		title,
		description,
		side = 'right',
		closeOnBackdrop = true,
		onclose,
		children,
		footer
	}: DrawerProps = $props();

	const id = $props.id();
	const titleId = `${id}-title`;
	const descriptionId = `${id}-description`;

	/** The panel's width (`w-96`), so it slides fully off its edge. */
	const offset = $derived(side === 'right' ? 384 : -384);

	function close() {
		open = false;
		onclose?.();
	}

	$effect(() => {
		if (!open) return;
		return pushOverlay(close);
	});
</script>

{#if open}
	<div class={['fixed inset-0', layers.overlay]}>
		<Backdrop onclick={closeOnBackdrop ? close : undefined} />
		<div
			role="dialog"
			aria-modal="true"
			aria-labelledby={titleId}
			aria-describedby={description ? descriptionId : undefined}
			tabindex="-1"
			class={[
				'fixed inset-y-0 flex w-96 max-w-full flex-col border-border bg-surface shadow-lg outline-none',
				side === 'right' ? 'right-0 border-l' : 'left-0 border-r'
			]}
			{@attach trapFocus}
			in:fly={{ x: offset, duration: motionDuration('slow'), easing: easings.enter }}
			out:fly={{ x: offset, duration: motionDuration('slow'), easing: easings.exit }}
		>
			<OverlayHeader {title} {titleId} {description} {descriptionId} onclose={close} />
			<div class="min-h-0 flex-1 overflow-y-auto px-4 pb-4 text-sm text-fg">
				{@render children?.()}
			</div>
			{#if footer}
				<footer class="flex justify-end gap-2 border-t border-border p-4">
					{@render footer()}
				</footer>
			{/if}
		</div>
	</div>
{/if}
