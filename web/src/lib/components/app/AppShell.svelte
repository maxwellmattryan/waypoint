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
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { Badge, NavLink } from '#lib/components/ui';

	let { connection, children }: AppShellProps = $props();

	const badge = $derived(connectionBadges[connection]);

	const dashboardHref = resolve('/');
	const applicationsHref = resolve('/applications');
	const goalsHref = resolve('/goals');
	const postingsHref = resolve('/postings');
	const resumesHref = resolve('/resumes');
	const memoriesHref = resolve('/memories');
	const reviewHref = resolve('/review');
	const runsHref = resolve('/runs');
	const path = $derived(page.url.pathname);
	// A section's detail pages (`/applications/<id>`, `/runs/<id>`, …) belong to it too.
	const inSection = (href: string) => path === href || path.startsWith(`${href}/`);
</script>

<div class="flex min-h-screen flex-col">
	<header class="border-b border-border bg-surface">
		<div
			class="mx-auto flex min-h-16 max-w-5xl flex-wrap items-center gap-x-4 gap-y-2 px-4 py-2 sm:flex-nowrap sm:px-8"
		>
			<p class="text-lg font-semibold tracking-tight text-fg">Waypoint</p>
			<nav
				aria-label="Main"
				class="order-last flex w-full flex-wrap items-center gap-1 sm:order-none sm:w-auto sm:flex-nowrap"
			>
				<NavLink href={dashboardHref} current={path === dashboardHref}>Dashboard</NavLink>
				<NavLink href={applicationsHref} current={inSection(applicationsHref)}>Applications</NavLink>
				<NavLink href={goalsHref} current={inSection(goalsHref)}>Goals</NavLink>
				<NavLink href={postingsHref} current={inSection(postingsHref)}>Postings</NavLink>
				<NavLink href={resumesHref} current={inSection(resumesHref)}>Resumes</NavLink>
				<NavLink href={memoriesHref} current={inSection(memoriesHref)}>Memory</NavLink>
				<NavLink href={reviewHref} current={inSection(reviewHref)}>Review</NavLink>
				<NavLink href={runsHref} current={inSection(runsHref)}>Runs</NavLink>
			</nav>
			<Badge
				tone={badge.tone}
				role="status"
				aria-label="Live updates: {badge.label}"
				class="ml-auto"
			>
				{badge.label}
			</Badge>
		</div>
	</header>
	<main class="mx-auto w-full max-w-5xl flex-1 px-4 py-8 sm:px-8">
		{@render children()}
	</main>
</div>
