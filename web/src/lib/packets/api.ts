import type {
	ApiClient,
	ApplicationPacket,
	FitRun,
	PacketQueued,
	PacketStatus,
	PostingPackets
} from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';
import type { BadgeTone } from '#lib/components/ui';

export type { Result };

/** The `prep_packet` workflow's name, as `run.updated` events carry it. */
export const PREP_PACKET_WORKFLOW = 'prep_packet';

/** How many questions a packet answers. */
export const QUESTION_COUNT = 5;

/** How each packet status reads on a badge. */
export const PACKET_STATUS_LABELS: Record<PacketStatus, string> = {
	ready: 'Ready for approval',
	in_review: 'Needs review',
	dismissed: 'Dismissed'
};

/** Each packet status's badge tone. */
export const PACKET_STATUS_TONES: Record<PacketStatus, BadgeTone> = {
	ready: 'success',
	in_review: 'warning',
	dismissed: 'neutral'
};

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
