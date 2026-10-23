<script lang="ts" module>
	import type { ApplicationPacket, PendingAction } from '#lib/api';
	import type { ApprovalInput, Result } from '#lib/packets/api.js';
	import type { PacketAction, ResumesState } from '#lib/packets/packets.svelte.js';

	export interface PacketDetailViewProps {
		packet: ApplicationPacket;
		/** The action in flight, if any. */
		busy?: PacketAction | null;
		/** The resumes the edit form offers. */
		resumes?: ResumesState;
		/** Loads the resumes, when the edit form opens and on Try again. */
		onloadresumes?: () => void;
		/** Approves the pending action: the draft without `input`, the edits with it. */
		onapprove?: (input?: ApprovalInput) => Promise<Result<PendingAction> | null>;
		onreject?: () => Promise<unknown>;
		onrequest?: () => void;
		onapply?: () => void;
		onundo?: () => void;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { fade, slide } from 'svelte/transition';
	import { formatDateTime } from '#lib/applications/format.js';
	import FitBadge from '#lib/components/postings/FitBadge.svelte';
	import RunLink from '#lib/components/review/RunLink.svelte';
	import { Badge, Card, Link } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import { confidenceLabel } from '#lib/postings/fit.js';
	import { formatCost } from '#lib/runs/format.js';
	import PacketActions from './PacketActions.svelte';
	import PacketAnswerEditor from './PacketAnswerEditor.svelte';
	import PacketAnswerList from './PacketAnswerList.svelte';
	import PacketStatusBadge from './PacketStatusBadge.svelte';
	import ResumeChoice from './ResumeChoice.svelte';

	/**
	 * One application packet: the header, the Actions card, the fit snapshot, the resume with its
	 * reasons and the answers with their citations. An approved or applied packet shows its
	 * approved version; Edit turns the resume and answers into a form. Nothing is submitted.
	 */
	let {
		packet,
		busy = null,
		resumes = { kind: 'idle' },
		onloadresumes = () => {},
		onapprove = async () => null,
		onreject = async () => {},
		onrequest = () => {},
		onapply = () => {},
		onundo = () => {}
	}: PacketDetailViewProps = $props();

	let editing = $state(false);

	const postingHref = $derived(resolve('/postings/[id]', { id: String(packet.posting_id) }));
	const reviewHref = resolve('/review');
	// The form only makes sense while the approval waits for the user.
	const showEditor = $derived(
		editing && packet.status === 'ready' && packet.pending_action_id !== null
	);
	const approvedVersion = $derived(
		(packet.status === 'approved' || packet.status === 'applied') &&
			packet.approved_answers !== null &&
			packet.approved_resume_name !== null
	);
	const resume = $derived(
		approvedVersion
			? {
					id: packet.approved_resume_version_id,
					name: packet.approved_resume_name ?? packet.resume_name,
					// The reasons explain the drafted resume, so only an unchanged choice keeps them.
					reasons:
						packet.approved_resume_name === packet.resume_name &&
						packet.approved_resume_version_id === packet.resume_version_id
							? packet.resume_reasons
							: []
				}
			: { id: packet.resume_version_id, name: packet.resume_name, reasons: packet.resume_reasons }
	);
	const answers = $derived(
		approvedVersion ? (packet.approved_answers ?? packet.answers) : packet.answers
	);

	function edit() {
		editing = true;
		onloadresumes();
	}

	async function approveEdits(input: ApprovalInput) {
		const result = await onapprove(input);
		if (result?.kind === 'ok' || result?.kind === 'conflict') editing = false;
		return result;
	}
</script>

<div class="flex flex-col gap-8">
	<header class="flex flex-col gap-2">
		<h1 class="text-2xl font-semibold text-fg">Application packet</h1>
		<Link href={postingHref} class="self-start text-base break-words">
			{packet.company} — {packet.title}
		</Link>
		<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
			<PacketStatusBadge status={packet.status} />
			<span class="text-xs text-fg-muted tabular-nums">{confidenceLabel(packet.confidence)}</span>
		</div>
		<p class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-fg-muted">
			<span class="tabular-nums">
				Prepared {formatDateTime(packet.created_at)} · {formatCost(packet.cost_usd)}
			</span>
			<RunLink runId={packet.run_id} />
		</p>
	</header>

	{#if packet.status === 'in_review'}
		<div
			class="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-md bg-warning-soft px-4 py-2 text-sm text-warning-soft-fg"
			transition:slide={{ duration: motionDuration('base'), easing: easings.standard }}
		>
			<span>This packet is waiting in review</span>
			<Link href={reviewHref}>Open the review queue</Link>
		</div>
	{/if}

	<section class="flex flex-col gap-4" aria-labelledby="packet-actions-heading">
		<h2 id="packet-actions-heading" class="text-lg font-semibold text-fg">Actions</h2>
		<PacketActions
			{packet}
			{busy}
			editing={showEditor}
			onapprove={() => void onapprove()}
			onedit={edit}
			{onreject}
			{onrequest}
			{onapply}
			{onundo}
		/>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="packet-fit-heading">
		<h2 id="packet-fit-heading" class="text-lg font-semibold text-fg">Fit</h2>
		<Card>
			{#if packet.fit_score !== null && packet.fit_band !== null}
				<FitBadge score={packet.fit_score} band={packet.fit_band} />
			{:else}
				<p class="text-sm text-fg-muted">Not scored</p>
			{/if}
		</Card>
	</section>

	{#if showEditor}
		<section
			class="flex flex-col gap-4"
			aria-labelledby="packet-edit-heading"
			in:fade={{ duration: motionDuration('base') }}
		>
			<h2 id="packet-edit-heading" class="text-lg font-semibold text-fg">Edit resume and answers</h2>
			<Card>
				<PacketAnswerEditor
					{packet}
					{resumes}
					onretry={onloadresumes}
					onsubmit={approveEdits}
					oncancel={() => (editing = false)}
				/>
			</Card>
		</section>
	{:else}
		<div class="flex flex-col gap-8" in:fade={{ duration: motionDuration('base') }}>
			<section class="flex flex-col gap-4" aria-labelledby="packet-resume-heading">
				<div class="flex flex-wrap items-center gap-2">
					<h2 id="packet-resume-heading" class="text-lg font-semibold text-fg">Resume</h2>
					{#if approvedVersion}
						<Badge tone="info">Approved version</Badge>
					{/if}
				</div>
				<Card>
					<ResumeChoice resumeId={resume.id} name={resume.name} reasons={resume.reasons} />
				</Card>
			</section>

			<section class="flex flex-col gap-4" aria-labelledby="packet-answers-heading">
				<div class="flex flex-wrap items-center gap-2">
					<h2 id="packet-answers-heading" class="text-lg font-semibold text-fg">Answers</h2>
					{#if approvedVersion}
						<Badge tone="info">Approved version</Badge>
					{/if}
				</div>
				<Card>
					<PacketAnswerList {answers} />
				</Card>
			</section>
		</div>
	{/if}
</div>
