<script lang="ts" module>
	import type { BadgeTone } from '#lib/components/ui';
	import type { GoogleState } from '#lib/dashboard/dashboard.svelte.js';
	import { API_PREFIX } from '#lib/config.js';

	export interface GoogleCardProps {
		google: GoogleState;
		/** Starts the OAuth flow. Defaults to navigating to the API's connect route. */
		onconnect?: () => void;
	}

	/** The API's OAuth start route, through the dev server's proxy. */
	export const GOOGLE_CONNECT_URL = `${API_PREFIX}/auth/google/connect`;

	const badges: Record<string, { tone: BadgeTone; label: string }> = {
		error: { tone: 'danger', label: 'Unavailable' },
		unconfigured: { tone: 'neutral', label: 'Not configured' },
		disconnected: { tone: 'neutral', label: 'Not connected' },
		connected: { tone: 'success', label: 'Connected' },
		reconnect_required: { tone: 'warning', label: 'Reconnect required' }
	};
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { Badge, Button, Skeleton } from '#lib/components/ui';
	import { scopeLabel } from '#lib/dashboard/job.js';
	import { motionDuration } from '#lib/motion.js';
	import DashboardCard from './DashboardCard.svelte';

	let {
		google,
		onconnect = () => window.location.assign(GOOGLE_CONNECT_URL)
	}: GoogleCardProps = $props();

	/** One key per visible state: `loading`, `error`, `unconfigured` or the connection state. */
	const view = $derived(
		google.kind !== 'loaded'
			? google.kind
			: google.status.configured
				? google.status.state
				: 'unconfigured'
	);
</script>

<DashboardCard title="Google">
	{#snippet status()}
		{#if view !== 'loading'}
			<Badge tone={badges[view].tone} data-testid="google-status">{badges[view].label}</Badge>
		{/if}
	{/snippet}

	{#key view}
		<div class="flex flex-col gap-4" in:fade={{ duration: motionDuration('base') }}>
			{#if google.kind === 'loading'}
				<div class="flex flex-col gap-4" data-testid="google-loading">
					<Skeleton shape="text" class="w-full" />
					<Skeleton shape="text" class="w-2/3" />
				</div>
			{:else if google.kind === 'error'}
				<p class="text-sm text-fg-muted">Couldn't load the Google status.</p>
			{:else if !google.status.configured}
				<p class="text-sm text-fg-muted">
					Set <code class="font-mono text-xs">WAYPOINT_GOOGLE_CLIENT_ID</code>,
					<code class="font-mono text-xs">WAYPOINT_GOOGLE_CLIENT_SECRET</code> and
					<code class="font-mono text-xs">WAYPOINT_TOKEN_KEY</code> in
					<code class="font-mono text-xs">.env</code> to connect Gmail and Calendar. See
					<code class="font-mono text-xs">docs/google-oauth.md</code>.
				</p>
				<div><Button variant="secondary" disabled>Connect Google</Button></div>
			{:else if google.status.state === 'connected'}
				{#if google.status.connected_at}
					<p class="text-sm text-fg-muted">
						Connected <time datetime={google.status.connected_at}
							>{new Date(google.status.connected_at).toLocaleString()}</time
						>
					</p>
				{/if}
				{#if google.status.scopes.length}
					<ul class="flex flex-wrap gap-2" aria-label="Granted access">
						{#each google.status.scopes as scope (scope)}
							<li><Badge>{scopeLabel(scope)}</Badge></li>
						{/each}
					</ul>
				{/if}
			{:else if google.status.state === 'reconnect_required'}
				<p class="text-sm text-fg-muted">
					Google stopped accepting Waypoint's access. Reconnect to keep reading Gmail and
					Calendar.
				</p>
				<div><Button onclick={onconnect}>Reconnect Google</Button></div>
			{:else}
				<p class="text-sm text-fg-muted">Connect a Google account to read Gmail and Calendar.</p>
				<div><Button onclick={onconnect}>Connect Google</Button></div>
			{/if}
		</div>
	{/key}
</DashboardCard>
