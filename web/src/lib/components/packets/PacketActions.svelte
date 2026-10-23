<script lang="ts" module>
	import type { ApplicationPacket } from '#lib/api';
	import type { PacketAction } from '#lib/packets/packets.svelte.js';

	export interface PacketActionsProps {
		packet: ApplicationPacket;
		/** The action in flight, if any: every button is disabled and the clicked one says so. */
		busy?: PacketAction | null;
		/** Whether the edit form is open, so Approve and Edit wait for it. */
		editing?: boolean;
		/** Approves the draft as it is. */
		onapprove: () => void;
		/** Opens the edit form. */
		onedit: () => void;
		/** Rejects the approval, once the user confirmed it. */
		onreject: () => Promise<unknown>;
		/** Requests an approval. */
		onrequest: () => void;
		/** Marks the approved packet applied. */
		onapply: () => void;
		/** Undoes the approval or the mark applied. */
		onundo: () => void;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { fade } from 'svelte/transition';
	import { formatDateTime } from '#lib/applications/format.js';
	import RunLink from '#lib/components/review/RunLink.svelte';
	import { Button, Card, ConfirmDialog, Link } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';
	import { actionState } from '#lib/packets/api.js';

	/**
	 * The packet page's Actions card: what the user can do with the packet in its current state
	 * (see docs/application-prep.md, "Packet page"). Nothing here submits anything.
	 */
	let {
		packet,
		busy = null,
		editing = false,
		onapprove,
		onedit,
		onreject,
		onrequest,
		onapply,
		onundo
	}: PacketActionsProps = $props();

	const view = $derived(actionState(packet));
	const sending = $derived(busy !== null);
	let confirming = $state(false);

	const reviewHref = resolve('/review');
	const applicationHref = (id: number) => resolve('/applications/[id]', { id: String(id) });
</script>

<Card>
	{#key view.kind}
		<div class="flex flex-col gap-4" in:fade={{ duration: motionDuration('base') }}>
			{#if view.kind === 'inReview'}
				<p class="text-sm text-fg">Accept or dismiss its review item before approving.</p>
				<Link href={reviewHref} class="self-start text-sm">Go to Review</Link>
			{:else if view.kind === 'awaitingApproval'}
				<p class="text-sm text-fg-muted">
					Approve the packet as it is, or edit the resume and answers first.
				</p>
				<div class="flex flex-wrap gap-2">
					<Button disabled={sending || editing} onclick={onapprove}>
						{busy === 'approve' ? 'Approving…' : 'Approve'}
					</Button>
					<Button variant="secondary" disabled={sending || editing} onclick={onedit}>Edit</Button>
					<Button variant="danger" disabled={sending} onclick={() => (confirming = true)}>
						{busy === 'reject' ? 'Rejecting…' : 'Reject'}
					</Button>
				</div>
			{:else if view.kind === 'requesting'}
				<p class="text-sm text-fg-muted" role="status">
					{view.paused ? 'Paused: agents are paused' : 'Getting the approval ready…'}
				</p>
			{:else if view.kind === 'requestable'}
				{#if view.failed}
					<div class="flex flex-col gap-1">
						<p class="text-sm font-medium text-danger">Couldn't request approval</p>
						{#if view.failed.error}
							<p class="text-sm break-words text-fg-muted">{view.failed.error}</p>
						{/if}
						<RunLink runId={view.failed.id} />
					</div>
				{/if}
				<Button class="self-start" disabled={sending} onclick={onrequest}>
					{busy === 'request' ? 'Requesting…' : 'Request approval'}
				</Button>
			{:else if view.kind === 'approved'}
				<p class="text-sm text-fg">
					Waypoint doesn't submit anything. Apply on the company's site, then mark it applied.
				</p>
				{#if view.resumeDeleted}
					<p class="text-sm text-warning-soft-fg">
						The approved resume was deleted. Undo the approval to choose another.
					</p>
				{/if}
				{#if view.applying}
					<p class="text-sm text-fg-muted" role="status">
						{view.applying.paused ? 'Paused: agents are paused' : 'Marking applied…'}
					</p>
				{:else if view.failed}
					<div class="flex flex-col gap-1">
						{#if view.failed.error}
							<p class="text-sm break-words text-danger">{view.failed.error}</p>
						{/if}
						<RunLink runId={view.failed.id} />
					</div>
				{/if}
				<div class="flex flex-wrap gap-2">
					<Button
						disabled={sending || view.resumeDeleted || view.applying !== null}
						onclick={onapply}
					>
						{busy === 'apply' ? 'Marking applied…' : view.failed ? 'Try again' : 'Mark applied'}
					</Button>
					<Button
						variant="secondary"
						disabled={sending || packet.undo_action_id === null}
						onclick={onundo}
					>
						{busy === 'undo' ? 'Undoing…' : 'Undo approval'}
					</Button>
				</div>
			{:else if view.kind === 'applied'}
				<p class="text-sm text-fg tabular-nums">
					{view.appliedAt ? `Applied ${formatDateTime(view.appliedAt)}` : 'Applied'}
				</p>
				<div class="flex flex-wrap items-center gap-4">
					{#if view.applicationId !== null}
						<Link href={applicationHref(view.applicationId)} class="text-sm">
							View application
						</Link>
					{:else}
						<span class="text-sm text-fg-muted">Application deleted</span>
					{/if}
					<Button
						variant="secondary"
						disabled={sending || packet.undo_action_id === null}
						onclick={onundo}
					>
						{busy === 'undo' ? 'Undoing…' : 'Undo'}
					</Button>
				</div>
			{:else}
				<p class="text-sm text-fg-muted">This packet was dismissed.</p>
			{/if}
		</div>
	{/key}
</Card>

<ConfirmDialog
	bind:open={confirming}
	title="Reject this packet?"
	description="It will be dismissed, and Waypoint learns from the rejection."
	confirmLabel="Reject"
	pendingLabel="Rejecting…"
	onconfirm={async () => {
		await onreject();
		return true;
	}}
/>
