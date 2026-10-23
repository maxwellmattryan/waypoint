<script lang="ts" module>
	import type { ApplicationPacket } from '#lib/api';

	export interface PacketDetailViewProps {
		packet: ApplicationPacket;
	}
</script>

<script lang="ts">
	import { resolve } from '$app/paths';
	import { slide } from 'svelte/transition';
	import { formatDateTime } from '#lib/applications/format.js';
	import FitBadge from '#lib/components/postings/FitBadge.svelte';
	import RunLink from '#lib/components/review/RunLink.svelte';
	import { Card, Link } from '#lib/components/ui';
	import { easings, motionDuration } from '#lib/motion.js';
	import { confidenceLabel } from '#lib/postings/fit.js';
	import { formatCost } from '#lib/runs/format.js';
	import PacketAnswerList from './PacketAnswerList.svelte';
	import PacketStatusBadge from './PacketStatusBadge.svelte';
	import ResumeChoice from './ResumeChoice.svelte';

	/**
	 * One application packet, read-only: the header, the fit snapshot, the chosen resume with its
	 * reasons, and the answers with their citations. It has no edit, approve or submit control.
	 */
	let { packet }: PacketDetailViewProps = $props();

	const postingHref = $derived(resolve('/postings/[id]', { id: String(packet.posting_id) }));
	const reviewHref = resolve('/review');
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

	<section class="flex flex-col gap-4" aria-labelledby="packet-resume-heading">
		<h2 id="packet-resume-heading" class="text-lg font-semibold text-fg">Resume</h2>
		<Card>
			<ResumeChoice
				resumeId={packet.resume_version_id}
				name={packet.resume_name}
				reasons={packet.resume_reasons}
			/>
		</Card>
	</section>

	<section class="flex flex-col gap-4" aria-labelledby="packet-answers-heading">
		<h2 id="packet-answers-heading" class="text-lg font-semibold text-fg">Answers</h2>
		<Card>
			<PacketAnswerList answers={packet.answers} />
		</Card>
	</section>
</div>
