import {
	createApiClient,
	type ApiClient,
	type ApplicationPacket,
	type Event,
	type PostingPackets
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { SCORE_FIT_WORKFLOW } from '#lib/postings/fit.js';
import {
	getPacket,
	getPostingPackets,
	packetsView,
	preparePacket,
	PREP_PACKET_WORKFLOW,
	type PacketsView
} from './api';

/** The posting a run's input names (`{"posting_id": id}`), or null. */
function inputPosting(input: unknown): number | null {
	if (typeof input !== 'object' || input === null) return null;
	const id = (input as Record<string, unknown>).posting_id;
	return typeof id === 'number' ? id : null;
}

export type PostingPacketsState =
	| { kind: 'loading' }
	| { kind: 'error' }
	| { kind: 'loaded'; packets: PostingPackets; view: PacketsView };

export interface PostingPacketsDeps {
	client: ApiClient;
	get: typeof getPostingPackets;
	prepare: typeof preparePacket;
}

/**
 * The posting page's Packets section: `GET /postings/{id}/packets`, kept current by `/events`.
 * The skeleton shows until the first load; later reloads keep what's shown, and a response
 * older than the latest request is dropped.
 */
export class PostingPacketsStore {
	state = $state<PostingPacketsState>({ kind: 'loading' });
	/** A `POST /postings/{id}/packet` is in flight. */
	starting = $state(false);

	readonly id: number;
	#deps: PostingPacketsDeps;
	#request = 0;

	constructor(id: number, deps: Partial<PostingPacketsDeps> = {}) {
		this.id = id;
		this.#deps = {
			client: deps.client ?? createApiClient(),
			get: deps.get ?? getPostingPackets,
			prepare: deps.prepare ?? preparePacket
		};
	}

	/** Loads the packets. A failure shows the error state until a load succeeds. */
	async load(): Promise<void> {
		const request = ++this.#request;
		const result = await this.#deps.get(this.#deps.client, this.id);
		if (request !== this.#request) return;
		if (result.kind === 'ok') this.#show(result.data);
		else this.state = { kind: 'error' };
	}

	/** Queues a packet (or rejoins the waiting run) and shows its run as preparing at once. */
	async prepare(): Promise<void> {
		if (this.starting) return;
		this.starting = true;
		try {
			const result = await this.#deps.prepare(this.#deps.client, this.id);
			if (result.kind !== 'ok') {
				toast.error("Couldn't start preparing", {
					description:
						result.kind === 'conflict'
							? 'Score this posting’s fit first.'
							: 'Check that the API is running, then try again.'
				});
				return;
			}
			// Any load in flight predates this run.
			this.#request++;
			const { id, status, error } = result.data.run;
			const run = { id, status, error };
			const packets: PostingPackets =
				this.state.kind === 'loaded'
					? { ...this.state.packets, run }
					: { posting_id: this.id, scored: true, packets: [], run };
			this.#show(packets);
		} finally {
			this.starting = false;
		}
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'run.updated':
				if (
					(event.run.workflow === PREP_PACKET_WORKFLOW ||
						event.run.workflow === SCORE_FIT_WORKFLOW) &&
					inputPosting(event.run.input) === this.id
				) {
					void this.load();
				}
				return;
			case 'app.connected':
			case 'app.lagged':
			case 'data.deleted':
				void this.load();
				return;
		}
	}

	/** Drops any response in flight. Call when the page goes. */
	destroy(): void {
		this.#request++;
	}

	#show(packets: PostingPackets): void {
		this.state = { kind: 'loaded', packets, view: packetsView(packets) };
	}
}

export type PacketState =
	| { kind: 'loading' }
	| { kind: 'not-found' }
	| { kind: 'error' }
	| { kind: 'loaded'; packet: ApplicationPacket };

export interface PacketDeps {
	client: ApiClient;
	get: typeof getPacket;
}

/**
 * One packet's page: `GET /packets/{id}`, reloaded when its run changes (review decisions
 * publish that) and after a reconnect, a lag or a data deletion. Later reloads keep what's
 * shown until they land; a response older than the latest request is dropped.
 */
export class PacketStore {
	state = $state<PacketState>({ kind: 'loading' });

	readonly id: number;
	#deps: PacketDeps;
	#request = 0;

	constructor(id: number, deps: Partial<PacketDeps> = {}) {
		this.id = id;
		this.#deps = {
			client: deps.client ?? createApiClient(),
			get: deps.get ?? getPacket
		};
	}

	/** Loads the packet. */
	async load(): Promise<void> {
		const request = ++this.#request;
		const result = await this.#deps.get(this.#deps.client, this.id);
		if (request !== this.#request) return;
		if (result.kind === 'ok') this.state = { kind: 'loaded', packet: result.data };
		else
			this.state = {
				kind: result.kind === 'not-found' ? 'not-found' : 'error'
			};
	}

	/** Applies one event from `/events`. */
	handle(event: Event): void {
		switch (event.type) {
			case 'run.updated':
				if (this.state.kind === 'loaded' && event.run.id === this.state.packet.run_id) {
					void this.load();
				}
				return;
			case 'app.connected':
			case 'app.lagged':
			case 'data.deleted':
				void this.load();
				return;
		}
	}

	/** Drops any response in flight. Call when the page goes. */
	destroy(): void {
		this.#request++;
	}
}
