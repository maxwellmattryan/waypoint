<script lang="ts">
	import { flip } from 'svelte/animate';
	import { fade, fly } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';
	import CloseIcon from './CloseIcon.svelte';
	import IconButton from './IconButton.svelte';
	import { toast } from './toast.svelte';
	import { layers, overlaySurface, toastTones } from './styles';

	// Renders the `toast` queue. Mount once, in the root layout.
</script>

<div
	aria-live="polite"
	class={[
		'pointer-events-none fixed right-4 bottom-4 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2',
		layers.toast
	]}
>
	{#each toast.items as item (item.id)}
		<div
			role={item.tone === 'danger' ? 'alert' : 'status'}
			data-tone={item.tone}
			class={[
				'pointer-events-auto relative flex items-start gap-2 overflow-hidden p-4',
				overlaySurface
			]}
			animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
			in:fly={{ y: 16, duration: motionDuration('base'), easing: easings.enter }}
			out:fade={{ duration: motionDuration('fast'), easing: easings.exit }}
		>
			<span class={['absolute inset-y-0 left-0 w-1', toastTones[item.tone]]} aria-hidden="true"
			></span>
			<div class="flex min-w-0 flex-1 flex-col gap-1 pt-1">
				<p class="text-sm font-medium text-fg">{item.title}</p>
				{#if item.description}
					<p class="text-sm text-fg-muted">{item.description}</p>
				{/if}
			</div>
			<IconButton label="Dismiss notification" size="sm" onclick={() => toast.dismiss(item.id)}>
				<CloseIcon />
			</IconButton>
		</div>
	{/each}
</div>
