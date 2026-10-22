<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export interface DashboardCardProps {
		title: string;
		/** Shown at the right of the title, e.g. a status badge. */
		status?: Snippet;
		children: Snippet;
	}
</script>

<script lang="ts">
	import { Card } from '#lib/components/ui';

	let { title, status, children }: DashboardCardProps = $props();
</script>

<Card>
	<section class="flex flex-col gap-4" aria-label={title}>
		<div class="flex min-h-6 items-center justify-between gap-4">
			<h2 class="text-base font-semibold text-fg">{title}</h2>
			{#if status}{@render status()}{/if}
		</div>
		{@render children()}
	</section>
</Card>
