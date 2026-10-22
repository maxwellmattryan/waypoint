<script lang="ts" module>
	import type { Snippet } from 'svelte';
	import type { BadgeTone } from '#lib/components/ui';
	import type { ConnectionState } from '#lib/dashboard/dashboard.svelte.js';

	export interface AppShellProps {
		/** The `/events` stream's state, shown as the live-connection badge. */
		connection: ConnectionState;
		children: Snippet;
	}

	const connectionBadges: Record<ConnectionState, { tone: BadgeTone; label: string }> = {
		connecting: { tone: 'neutral', label: 'Connecting' },
		live: { tone: 'success', label: 'Live' },
		reconnecting: { tone: 'warning', label: 'Reconnecting' }
	};
</script>

<script lang="ts">
	import { Badge } from '#lib/components/ui';

	let { connection, children }: AppShellProps = $props();

	const badge = $derived(connectionBadges[connection]);
</script>

<div class="flex min-h-screen flex-col">
	<header class="border-b border-border bg-surface">
		<div class="mx-auto flex h-16 max-w-5xl items-center justify-between gap-4 px-4 sm:px-8">
			<p class="text-lg font-semibold tracking-tight text-fg">Waypoint</p>
			<Badge tone={badge.tone} role="status" aria-label="Live updates: {badge.label}">
				{badge.label}
			</Badge>
		</div>
	</header>
	<main class="mx-auto w-full max-w-5xl flex-1 px-4 py-8 sm:px-8">
		{@render children()}
	</main>
</div>
