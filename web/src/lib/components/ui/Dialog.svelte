<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { DialogSize } from './styles';

	export interface DialogProps {
		/** Whether the dialog is shown. Bindable. */
		open?: boolean;
		/** Heading, and the dialog's accessible name. */
		title: string;
		/** Optional text under the title, linked with `aria-describedby`. */
		description?: string;
		size?: DialogSize;
		/** Close when the backdrop is clicked. Default `true`. */
		closeOnBackdrop?: boolean;
		/** Called after the dialog closes itself (Escape, Close button or backdrop). */
		onclose?: () => void;
		children?: Snippet;
		/** Actions row under the body. */
		footer?: Snippet;
	}
</script>

<script lang="ts">
	import { scale } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
	import Backdrop from './Backdrop.svelte';
	import OverlayHeader from './OverlayHeader.svelte';
	import { trapFocus } from './focus-trap';
	import { pushOverlay } from './overlay';
	import { dialogSizes, layers, overlaySurface } from './styles';

	let {
		open = $bindable(false),
		title,
		description,
		size = 'md',
		closeOnBackdrop = true,
		onclose,
		children,
		footer
	}: DialogProps = $props();

	const id = $props.id();
	const titleId = `${id}-title`;
	const descriptionId = `${id}-description`;

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
	<div class={['fixed inset-0 flex items-center justify-center p-4', layers.overlay]}>
		<Backdrop onclick={closeOnBackdrop ? close : undefined} />
		<div
			role="dialog"
			aria-modal="true"
			aria-labelledby={titleId}
			aria-describedby={description ? descriptionId : undefined}
			tabindex="-1"
			class={['relative flex max-h-full w-full flex-col outline-none', overlaySurface, dialogSizes[size]]}
			{@attach trapFocus}
			in:scale={{ start: 0.96, opacity: 0, duration: motionDuration('base'), easing: easings.enter }}
			out:scale={{ start: 0.96, opacity: 0, duration: motionDuration('base'), easing: easings.exit }}
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
