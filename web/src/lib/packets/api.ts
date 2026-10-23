import type {
	ApiClient,
	ApplicationPacket,
	FitRun,
	PacketQueued,
	PacketQuestion,
	PacketStatus,
	PostingPackets
} from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';
import type { BadgeTone } from '#lib/components/ui';

export type { Result };

/** The `prep_packet` workflow's name, as `run.updated` events carry it. */
export const PREP_PACKET_WORKFLOW = 'prep_packet';
/** The workflow that asks the user to approve a ready packet. */
export const APPROVE_PACKET_WORKFLOW = 'approve_packet';
/** The workflow that marks an approved packet applied. */
export const APPLY_PACKET_WORKFLOW = 'apply_packet';

/** How many questions a packet answers. */
export const QUESTION_COUNT = 5;
/** Most characters of an answer, after trimming. */
export const MAX_ANSWER = 1500;

/** How each packet status reads on a badge. */
export const PACKET_STATUS_LABELS: Record<PacketStatus, string> = {
	ready: 'Ready for approval',
	in_review: 'Needs review',
	dismissed: 'Dismissed',
	approved: 'Approved',
	applied: 'Applied'
};

/** Each packet status's badge tone. */
export const PACKET_STATUS_TONES: Record<PacketStatus, BadgeTone> = {
	ready: 'success',
	in_review: 'warning',
	dismissed: 'neutral',
	approved: 'info',
	applied: 'success'
};

/** What a packet's approval asks for: its draft, or the user's edits of it. */
export interface ApprovalInput {
	packet_id: number;
	/** `null` when no resume is chosen (the draft's was deleted). */
	resume_version_id: number | null;
	/** Every question's answer; `null` for none. */
	answers: Record<PacketQuestion, string | null>;
	[key: string]: unknown;
}

/** The approval request for `packet`'s draft: its resume and its answers. */
export function approvalInput(packet: ApplicationPacket): ApprovalInput {
	const answers = {} as Record<PacketQuestion, string | null>;
	for (const answer of packet.answers) answers[answer.key] = answer.answer;
	return {
		packet_id: packet.id,
		resume_version_id: packet.resume_version_id,
		answers
	};
}

/**
 * Queues the `approve_packet` run that asks to approve a ready packet, or gets back the one
 * already waiting. A packet that isn't `ready` is `conflict`.
 */
export function requestApproval(client: ApiClient, id: number): Promise<Result<PacketQueued>> {
	return run(202, () =>
		client.POST('/packets/{id}/approval', {
			params: { path: { id } },
			body: {}
		})
	);
}

/**
 * Queues the `apply_packet` run that records the user applied with an approved packet, or gets
 * back the one already waiting or running. Nothing is submitted. A packet that isn't
 * `approved`, or whose approved resume was deleted, is `conflict`.
 */
export function markApplied(client: ApiClient, id: number): Promise<Result<PacketQueued>> {
	return run(202, () =>
		client.POST('/packets/{id}/applied', {
			params: { path: { id } },
			body: {}
		})
	);
}

/** What the packet page's Actions card shows, by packet state. */
export type PacketActionState =
	| { kind: 'inReview' }
	/** Approve, Edit and Reject act on this pending action. */
	| { kind: 'awaitingApproval'; actionId: number }
	/** The approval run is on its way to asking (`paused` while agents are paused). */
	| { kind: 'requesting'; paused: boolean }
	/** Request approval, with the failed approval run when there is one. */
	| { kind: 'requestable'; failed: FitRun | null }
	| {
			kind: 'approved';
			/** The approved resume was deleted, so it can't be marked applied. */
			resumeDeleted: boolean;
			/** A mark-applied run is waiting or running (`paused` while agents are paused). */
			applying: { paused: boolean } | null;
			/** The mark-applied run that failed, if the latest one did. */
			failed: FitRun | null;
	  }
	| { kind: 'applied'; appliedAt: string | null; applicationId: number | null }
	| { kind: 'dismissed' };

const isWaiting = (run: FitRun | null): run is FitRun =>
	run !== null && (run.status === 'queued' || run.status === 'running' || run.status === 'paused');

/** The Actions card's state for `packet` (see docs/application-prep.md, "Packet page"). */
export function actionState(packet: ApplicationPacket): PacketActionState {
	const latest = packet.latest_run;
	const failed = latest !== null && latest.status === 'failed' ? latest : null;
	switch (packet.status) {
		case 'in_review':
			return { kind: 'inReview' };
		case 'dismissed':
			return { kind: 'dismissed' };
		case 'applied':
			return {
				kind: 'applied',
				appliedAt: packet.applied_at,
				applicationId: packet.application_id
			};
		case 'approved':
			return {
				kind: 'approved',
				resumeDeleted: packet.approved_resume_version_id === null,
				applying: isWaiting(latest) ? { paused: latest.status === 'paused' } : null,
				failed
			};
		case 'ready':
			if (packet.pending_action_id !== null)
				return { kind: 'awaitingApproval', actionId: packet.pending_action_id };
			if (isWaiting(latest)) return { kind: 'requesting', paused: latest.status === 'paused' };
			return { kind: 'requestable', failed };
	}
}

/** A posting's packets and latest prep run; a 404 is `not-found`. */
export function getPostingPackets(client: ApiClient, id: number): Promise<Result<PostingPackets>> {
	return run(200, () => client.GET('/postings/{id}/packets', { params: { path: { id } } }));
}

/**
 * Queues a `prep_packet` run for a posting, or gets back the one already waiting or running. A
 * posting without an applied fit score is `conflict`.
 */
export function preparePacket(client: ApiClient, id: number): Promise<Result<PacketQueued>> {
	return run(202, () =>
		client.POST('/postings/{id}/packet', {
			params: { path: { id } },
			body: {}
		})
	);
}

/** One packet; a 404 is `not-found`. */
export function getPacket(client: ApiClient, id: number): Promise<Result<ApplicationPacket>> {
	return run(200, () => client.GET('/packets/{id}', { params: { path: { id } } }));
}

/** How many of a packet's questions have an answer. */
export function answeredCount(packet: ApplicationPacket): number {
	return packet.answers.filter((answer) => answer.answer !== null).length;
}

/** What the posting page's Packets section shows. */
export type PacketsView =
	| { kind: 'notScored' }
	| { kind: 'empty' }
	| { kind: 'preparing'; run: FitRun }
	| { kind: 'paused'; run: FitRun }
	| { kind: 'failed'; run: FitRun }
	/**
	 * The packets, newest first, with a waiting or running run (`preparing`, paused included)
	 * or a failed run newer than every packet (`failed`) shown above them.
	 */
	| {
			kind: 'list';
			packets: ApplicationPacket[];
			preparing: FitRun | null;
			failed: FitRun | null;
	  };

/**
 * The Packets section's state: the list when there are packets (with a preparing or failed run
 * above it), else a waiting or running run, else a failed run, else not scored or empty.
 */
export function packetsView(p: PostingPackets): PacketsView {
	const latest = p.run;
	const active =
		latest &&
		(latest.status === 'queued' || latest.status === 'running' || latest.status === 'paused')
			? latest
			: null;
	const newest = Math.max(0, ...p.packets.map((packet) => packet.run_id));
	const failed = latest && latest.status === 'failed' && newest < latest.id ? latest : null;
	if (p.packets.length > 0) {
		return { kind: 'list', packets: p.packets, preparing: active, failed };
	}
	if (active)
		return {
			kind: active.status === 'paused' ? 'paused' : 'preparing',
			run: active
		};
	if (failed) return { kind: 'failed', run: failed };
	if (!p.scored) return { kind: 'notScored' };
	return { kind: 'empty' };
}
