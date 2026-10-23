<script lang="ts" module>
	import type { PlanPacket } from '#lib/api';

	export interface PlanPacketListProps {
		/** The prep step's postings, in shortlist order. */
		packets: PlanPacket[];
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import PacketStatusBadge from '#lib/components/packets/PacketStatusBadge.svelte';
	import RunLink from '#lib/components/review/RunLink.svelte';
	import { Link } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';

	/** A goal's packets: each posting with its packet's status and a link to review it, or why there is none. */
	let { packets }: PlanPacketListProps = $props();
</script>

<ol class="flex flex-col divide-y divide-border" aria-label="Packets">
	{#each packets as entry (entry.posting_id)}
		<li
			class="flex flex-col gap-2 py-2 sm:flex-row sm:items-center sm:justify-between sm:gap-4"
			animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
			in:fade={{ duration: motionDuration('base') }}
		>
			<span class="flex min-w-0 flex-col gap-1">
				<Link href={resolve('/postings/[id]', { id: String(entry.posting_id) })} class="text-sm break-words">
					{entry.title}
				</Link>
				<span class="text-xs break-words text-fg-muted">{entry.company}</span>
			</span>
			<div class="flex min-w-0 flex-wrap items-center gap-2 sm:justify-end">
				{#if entry.packet_id !== null && entry.status !== null}
					<PacketStatusBadge status={entry.status} />
					<Link href={resolve('/packets/[id]', { id: String(entry.packet_id) })} class="text-sm">
						{entry.status === 'ready' ? 'Review and approve' : 'View packet'}
					</Link>
				{:else if entry.packet_id !== null}
					<span class="text-sm text-fg-muted">Packet removed</span>
				{:else if entry.outcome === 'queued'}
					<span class="text-sm text-fg-muted">Preparing…</span>
					{#if entry.run_id !== null}<RunLink runId={entry.run_id} />{/if}
				{:else}
					<span class="flex min-w-0 flex-col gap-1 sm:items-end">
						<span class="flex flex-wrap items-center gap-2">
							<span class="text-sm text-danger">Couldn't prepare</span>
							{#if entry.run_id !== null}<RunLink runId={entry.run_id} />{/if}
						</span>
						{#if entry.error}
							<span class="text-xs break-words text-fg-muted">{entry.error}</span>
						{/if}
					</span>
				{/if}
			</div>
		</li>
	{/each}
</ol>
