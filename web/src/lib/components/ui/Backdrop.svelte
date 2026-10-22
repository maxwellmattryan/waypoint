<script lang="ts">
	import { fade } from 'svelte/transition';
	import { easings, motionDuration } from '#lib/motion.js';

	// The dimmed layer behind a Dialog or Drawer. Internal: not exported from the barrel.
	let { onclick }: { onclick?: (event: MouseEvent) => void } = $props();
</script>

<!-- The backdrop is a mouse convenience; keyboard users close with Escape or the Close button. -->
<div
	class="fixed inset-0 bg-overlay"
	aria-hidden="true"
	data-backdrop
	{onclick}
	in:fade={{ duration: motionDuration('base'), easing: easings.enter }}
	out:fade={{ duration: motionDuration('base'), easing: easings.exit }}
></div>
