<script lang="ts" module>
	import type { SyncMode } from '#lib/api';
	import type { GmailSyncState, GoogleState } from '#lib/dashboard/dashboard.svelte.js';

	export interface GmailSyncCardProps {
		gmailSync: GmailSyncState;
		/** The Google connection; syncing needs a configured, connected account. */
		google: GoogleState;
		/** Whether a "Sync now" request is in flight. */
		syncing: boolean;
		onsync: () => void;
	}

	/** Mirrors the API's default backfill window (`gmail::Config::backfill_days`). */
	const BACKFILL_DAYS = 30;

	const modes: Record<SyncMode, string> = {
		backfill: `Backfilling the last ${BACKFILL_DAYS} days`,
		incremental: 'Catching up',
		resync: 'Resyncing (history expired)'
	};
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { Badge, Button, KeyValue, Skeleton } from '#lib/components/ui';
	import { formatDateTime } from '#lib/applications/format.js';
	import { motionDuration } from '#lib/motion.js';
	import DashboardCard from './DashboardCard.svelte';

	let { gmailSync, google, syncing, onsync }: GmailSyncCardProps = $props();

	const connected = $derived(
		google.kind === 'loaded' && google.status.configured && google.status.state === 'connected'
	);
	const current = $derived(gmailSync.kind === 'loaded' ? gmailSync.status : null);

	/** One key per visible state. */
	const view = $derived(
		gmailSync.kind === 'loading' || google.kind === 'loading'
			? 'loading'
			: !current
				? 'error'
				: !connected
					? 'disconnected'
					: current.activity !== 'idle'
						? 'active'
						: current.last_finished_at
							? 'idle'
							: 'never'
	);

	const stored = $derived(current ? current.run.inserted + current.run.updated : 0);
	const skipped = $derived(current?.run.oversized ?? 0);
	const waitingUntil = $derived(
		current?.activity === 'queued' &&
			current.next_run_at &&
			new Date(current.next_run_at).getTime() > Date.now()
			? current.next_run_at
			: null
	);
	const canSync = $derived((view === 'idle' || view === 'never') && !syncing);
</script>

<DashboardCard title="Gmail">
	{#snippet status()}
		{#if view === 'active' && current}
			{#key current.activity}
				<span in:fade={{ duration: motionDuration('base') }}>
					{#if current.activity === 'running'}
						<Badge tone="info" data-testid="gmail-status">Syncing</Badge>
					{:else}
						<Badge tone="neutral" data-testid="gmail-status">Queued</Badge>
					{/if}
				</span>
			{/key}
		{/if}
	{/snippet}

	{#key view}
		<div class="flex flex-col gap-4" in:fade={{ duration: motionDuration('base') }}>
			{#if view === 'loading'}
				<div class="flex flex-col gap-4" data-testid="gmail-loading">
					<Skeleton shape="text" class="w-full" />
					<Skeleton shape="text" class="w-2/3" />
				</div>
			{:else if view === 'error' || !current}
				<p class="text-sm text-fg-muted">Couldn't load the Gmail sync status.</p>
			{:else}
				<div class="flex flex-col gap-2" aria-live="polite">
					{#if view === 'disconnected'}
						<p class="text-sm text-fg-muted">Connect Google to sync Gmail.</p>
					{:else if view === 'never'}
						<p class="text-sm text-fg-muted">Not synced yet.</p>
					{:else if view === 'active'}
						<p class="text-sm text-fg">{current.mode ? modes[current.mode] : 'Starting'}</p>
						{#if waitingUntil}
							<p class="text-sm text-fg-muted">
								Waiting for Gmail's rate limit until <time datetime={waitingUntil}
									>{formatDateTime(waitingUntil)}</time
								>
							</p>
						{/if}
					{:else if current.last_finished_at}
						<p class="text-sm text-fg-muted">
							Last synced <time datetime={current.last_finished_at}
								>{formatDateTime(current.last_finished_at)}</time
							>
						</p>
					{/if}
					{#if current.last_error}
						<p class="text-sm text-danger" data-testid="gmail-error">{current.last_error}</p>
					{/if}
				</div>
				{#if view === 'active'}
					<KeyValue
						items={[
							{ key: 'Stored', value: String(stored) },
							{ key: 'Skipped (too large)', value: String(skipped) }
						]}
						data-testid="gmail-counts"
					/>
				{:else if view === 'idle'}
					<KeyValue
						items={[
							{ key: 'Emails stored', value: String(current.emails_stored) },
							{ key: 'Last run', value: `${stored} stored, ${skipped} skipped` }
						]}
						data-testid="gmail-counts"
					/>
				{/if}
				<div><Button onclick={onsync} disabled={!canSync}>Sync now</Button></div>
			{/if}
		</div>
	{/key}
</DashboardCard>
