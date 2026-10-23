import {
	createApiClient,
	type ApiClient,
	type ApplicationPacket,
	type Event,
	type PendingAction,
	type PostingPackets,
	type ResumeVersion
} from '#lib/api';
import { toast } from '#lib/components/ui';
import { SCORE_FIT_WORKFLOW } from '#lib/postings/fit.js';
import { listResumes } from '#lib/resumes/api.js';
import { approveAction, rejectAction, undoAction } from '#lib/review/api.js';
import {
	APPLY_PACKET_WORKFLOW,
	APPROVE_PACKET_WORKFLOW,
	getPacket,
	getPostingPackets,
	markApplied,
	packetsView,
	preparePacket,
	PREP_PACKET_WORKFLOW,
	requestApproval,
	type ApprovalInput,
	type PacketsView,
	type Result
} from './api';

/** The number a run's input names under `key` (`{"posting_id": id}`), or null. */
function inputId(input: unknown, key: 'posting_id' | 'packet_id'): number | null {
	if (typeof input !== 'object' || input === null) return null;
	const id = (input as Record<string, unknown>)[key];
	return typeof id === 'number' ? id : null;
}

const inputPosting = (input: unknown) => inputId(input, 'posting_id');

/** The workflows whose runs change a posting's packets. */
const POSTING_WORKFLOWS: readonly string[] = [
	PREP_PACKET_WORKFLOW,
	SCORE_FIT_WORKFLOW,
	APPROVE_PACKET_WORKFLOW,
	APPLY_PACKET_WORKFLOW
];

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
					POSTING_WORKFLOWS.includes(event.run.workflow) &&
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
	approve: typeof approveAction;
	reject: typeof rejectAction;
	request: typeof requestApproval;
	apply: typeof markApplied;
	undo: typeof undoAction;
	resumes: typeof listResumes;
}

/** An action the packet page sends. */
export type PacketAction = 'approve' | 'reject' | 'request' | 'apply' | 'undo';

/** The resumes the edit form offers. */
export type ResumesState =
	| { kind: 'idle' }
	| { kind: 'loading' }
	| { kind: 'error' }
	| { kind: 'loaded'; resumes: ResumeVersion[] };

/** Each action's success toast and, after "Couldn't", its verb. */
const ACTIONS: Record<PacketAction, { done: string; verb: string }> = {
	approve: { done: 'Packet approved', verb: 'approve the packet' },
	reject: { done: 'Packet rejected', verb: 'reject the packet' },
	request: { done: 'Approval requested', verb: 'request approval' },
	apply: { done: 'Marking applied…', verb: 'mark it applied' },
	undo: { done: 'Undone', verb: 'undo' }
};

/**
 * One packet's page: `GET /packets/{id}`, reloaded when its run changes (review decisions
 * publish that), when an `approve_packet` or `apply_packet` run names it, and after a
 * reconnect, a lag or a data deletion. Later reloads keep what's shown until they land; a
 * response older than the latest request is dropped. It also sends the page's actions: one
 * at a time (`busy`), each reloading the packet when it lands.
 */
export class PacketStore {
	state = $state<PacketState>({ kind: 'loading' });
	/** The action in flight, if any. */
	busy = $state<PacketAction | null>(null);
	/** The edit form's resumes, loaded on request. */
	resumes = $state<ResumesState>({ kind: 'idle' });

	readonly id: number;
	#deps: PacketDeps;
	#request = 0;

	constructor(id: number, deps: Partial<PacketDeps> = {}) {
		this.id = id;
		this.#deps = {
			client: deps.client ?? createApiClient(),
			get: deps.get ?? getPacket,
			approve: deps.approve ?? approveAction,
			reject: deps.reject ?? rejectAction,
			request: deps.request ?? requestApproval,
			apply: deps.apply ?? markApplied,
			undo: deps.undo ?? undoAction,
			resumes: deps.resumes ?? listResumes
		};
	}

	get #packet(): ApplicationPacket | null {
		return this.state.kind === 'loaded' ? this.state.packet : null;
	}

	/**
	 * Approves the packet's pending action: the draft as it is, or `input` (the edited resume
	 * and answers). A 422 is returned without a toast, for the edit form to show.
	 */
	approve(input?: ApprovalInput): Promise<Result<PendingAction> | null> {
		const id = this.#packet?.pending_action_id ?? null;
		if (id === null) return Promise.resolve(null);
		return this.#act('approve', () => this.#deps.approve(this.#deps.client, id, input));
	}

	/** Rejects the packet's pending action; the packet is dismissed. */
	reject(): Promise<Result<PendingAction> | null> {
		const id = this.#packet?.pending_action_id ?? null;
		if (id === null) return Promise.resolve(null);
		return this.#act('reject', () => this.#deps.reject(this.#deps.client, id));
	}

	/** Queues the packet's approval request. */
	requestApproval(): Promise<Result<unknown> | null> {
		return this.#act('request', () => this.#deps.request(this.#deps.client, this.id));
	}

	/** Queues marking the approved packet applied. */
	markApplied(): Promise<Result<unknown> | null> {
		return this.#act('apply', () => this.#deps.apply(this.#deps.client, this.id));
	}

	/** Undoes the action behind the packet's current state (its approval or mark applied). */
	undo(): Promise<Result<unknown> | null> {
		const id = this.#packet?.undo_action_id ?? null;
		if (id === null) return Promise.resolve(null);
		return this.#act('undo', () => this.#deps.undo(this.#deps.client, id));
	}

	/** Loads the resumes the edit form offers. */
	async loadResumes(): Promise<void> {
		if (this.resumes.kind === 'loading') return;
		this.resumes = { kind: 'loading' };
		const result = await this.#deps.resumes(this.#deps.client);
		this.resumes =
			result.kind === 'ok' ? { kind: 'loaded', resumes: result.data } : { kind: 'error' };
	}

	/**
	 * Sends one action unless another is in flight: a success toasts and reloads, a 409 toasts
	 * that the packet changed and reloads, a 422 is left to the caller and anything else toasts
	 * that it failed.
	 */
	async #act<T>(action: PacketAction, send: () => Promise<Result<T>>): Promise<Result<T> | null> {
		if (this.busy !== null) return null;
		this.busy = action;
		try {
			const result = await send();
			const { done, verb } = ACTIONS[action];
			switch (result.kind) {
				case 'ok':
					toast.success(done);
					await this.load();
					break;
				case 'conflict':
					toast.error('This packet changed. It has been reloaded.');
					await this.load();
					break;
				case 'invalid':
					break;
				default:
					toast.error(`Couldn't ${verb}. Try again.`);
			}
			return result;
		} finally {
			this.busy = null;
		}
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
			case 'run.updated': {
				const own = this.state.kind === 'loaded' && event.run.id === this.state.packet.run_id;
				const acting =
					(event.run.workflow === APPROVE_PACKET_WORKFLOW ||
						event.run.workflow === APPLY_PACKET_WORKFLOW) &&
					inputId(event.run.input, 'packet_id') === this.id;
				if (own || acting) void this.load();
				return;
			}
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
