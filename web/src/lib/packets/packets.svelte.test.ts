import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	createApiClient,
	type ApplicationPacket,
	type Event,
	type PacketQueued,
	type PostingPackets,
	type Run
} from '#lib/api';
import { toast } from '#lib/components/ui';
import {
	makeApprovedPacket,
	makeFitRun,
	makePacket,
	makePostingPackets
} from './__fixtures__/packets';
import {
	actionState,
	answeredCount,
	approvalInput,
	packetsView,
	PACKET_STATUS_LABELS,
	PACKET_STATUS_TONES,
	type Result
} from './api';
import { PacketStore, PostingPacketsStore } from './packets.svelte';

const client = createApiClient({ baseUrl: 'http://api.test', fetch: vi.fn() });

afterEach(() => toast.clear());

const toasts = () => toast.items.map((t) => [t.tone, t.title]);

async function settle() {
	for (let i = 0; i < 5; i++) await Promise.resolve();
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

function makeRun(overrides: Partial<Run> = {}): Run {
	return {
		id: 60,
		workflow: 'prep_packet',
		status: 'queued',
		input: { posting_id: 9 },
		output: null,
		error: null,
		input_tokens: 0,
		output_tokens: 0,
		cost_usd: 0,
		job_id: 1,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		started_at: null,
		finished_at: null,
		...overrides
	};
}

const runUpdated = (overrides: Partial<Run> = {}): Event => ({
	type: 'run.updated',
	run: makeRun(overrides)
});

describe('packetsView', () => {
	const older = makePacket({ id: 2, run_id: 40 });
	const newer = makePacket({ id: 3, run_id: 50 });

	it.each([
		['not scored', makePostingPackets({ scored: false }), { kind: 'notScored' }],
		['empty', makePostingPackets(), { kind: 'empty' }],
		[
			'queued',
			makePostingPackets({ run: makeFitRun('queued', 60) }),
			{ kind: 'preparing', run: makeFitRun('queued', 60) }
		],
		[
			'running',
			makePostingPackets({ run: makeFitRun('running', 60) }),
			{ kind: 'preparing', run: makeFitRun('running', 60) }
		],
		[
			'paused',
			makePostingPackets({ run: makeFitRun('paused', 60) }),
			{ kind: 'paused', run: makeFitRun('paused', 60) }
		],
		[
			'failed',
			makePostingPackets({ run: makeFitRun('failed', 60, 'boom') }),
			{ kind: 'failed', run: makeFitRun('failed', 60, 'boom') }
		],
		[
			'a failed run on an unscored posting',
			makePostingPackets({
				scored: false,
				run: makeFitRun('failed', 60, 'boom')
			}),
			{ kind: 'failed', run: makeFitRun('failed', 60, 'boom') }
		],
		[
			'packets',
			makePostingPackets({
				packets: [newer, older],
				run: makeFitRun('succeeded', 50)
			}),
			{ kind: 'list', packets: [newer, older], preparing: null, failed: null }
		],
		[
			'packets with a run preparing',
			makePostingPackets({ packets: [newer], run: makeFitRun('running', 60) }),
			{
				kind: 'list',
				packets: [newer],
				preparing: makeFitRun('running', 60),
				failed: null
			}
		],
		[
			'packets with a newer failed run',
			makePostingPackets({
				packets: [newer],
				run: makeFitRun('failed', 60, 'boom')
			}),
			{
				kind: 'list',
				packets: [newer],
				preparing: null,
				failed: makeFitRun('failed', 60, 'boom')
			}
		],
		[
			'packets newer than a failed run',
			makePostingPackets({
				packets: [newer],
				run: makeFitRun('failed', 45, 'boom')
			}),
			{ kind: 'list', packets: [newer], preparing: null, failed: null }
		]
	] as [string, PostingPackets, ReturnType<typeof packetsView>][])('%s', (_, packets, view) => {
		expect(packetsView(packets)).toEqual(view);
	});

	it('counts answered questions', () => {
		const packet = makePacket();
		expect(answeredCount(packet)).toBe(5);
		packet.answers[2] = { ...packet.answers[2], answer: null, citations: [] };
		expect(answeredCount(packet)).toBe(4);
	});
});

describe('PostingPacketsStore', () => {
	const empty = makePostingPackets();

	function setup(...results: Result<PostingPackets>[]) {
		const get = vi.fn<(c: unknown, id: number) => Promise<Result<PostingPackets>>>();
		for (const result of results) get.mockResolvedValueOnce(result);
		get.mockResolvedValue({ kind: 'ok', data: empty });
		const prepare = vi.fn<(c: unknown, id: number) => Promise<Result<PacketQueued>>>();
		const store = new PostingPacketsStore(9, { client, get, prepare });
		return { store, get, prepare };
	}

	it('loads the packets, and shows the error state until a load succeeds', async () => {
		const { store, get } = setup({ kind: 'failed' });
		expect(store.state.kind).toBe('loading');
		await store.load();
		expect(get).toHaveBeenCalledWith(client, 9);
		expect(store.state).toEqual({ kind: 'error' });
		await store.load();
		expect(store.state).toEqual({
			kind: 'loaded',
			packets: empty,
			view: { kind: 'empty' }
		});
	});

	it('posts a prepare and shows the preparing state at once', async () => {
		const { store, prepare } = setup();
		await store.load();
		prepare.mockResolvedValueOnce({
			kind: 'ok',
			data: { run: makeRun(), reused: false }
		});
		const started = store.prepare();
		expect(store.starting).toBe(true);
		await started;
		expect(prepare).toHaveBeenCalledWith(client, 9);
		expect(store.starting).toBe(false);
		expect(store.state.kind === 'loaded' && store.state.view).toEqual({
			kind: 'preparing',
			run: { id: 60, status: 'queued', error: null }
		});
	});

	it.each(['failed', 'conflict'] as const)(
		'toasts when a prepare cannot start (%s)',
		async (kind) => {
			const { store, prepare } = setup();
			await store.load();
			prepare.mockResolvedValueOnce({ kind });
			await store.prepare();
			expect(store.state.kind === 'loaded' && store.state.view).toEqual({
				kind: 'empty'
			});
			expect(toasts()).toEqual([['danger', "Couldn't start preparing"]]);
		}
	);

	it('reloads on run.updated for its own prep_packet or score_fit run only', async () => {
		const { store, get } = setup();
		await store.load();
		store.handle(runUpdated({ input: { posting_id: 10 } }));
		store.handle(runUpdated({ workflow: 'classify_email', input: { posting_id: 9 } }));
		store.handle(runUpdated({ input: { email_id: 9 } }));
		await settle();
		expect(get).toHaveBeenCalledTimes(1);
		store.handle(runUpdated({ status: 'succeeded' }));
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		store.handle(runUpdated({ workflow: 'score_fit', status: 'succeeded' }));
		await settle();
		expect(get).toHaveBeenCalledTimes(3);
		store.handle({ type: 'app.connected' });
		store.handle({ type: 'app.lagged', skipped: 3 });
		store.handle({ type: 'data.deleted' });
		await settle();
		expect(get).toHaveBeenCalledTimes(6);
	});

	it('reloads on run.updated for approve_packet and apply_packet runs of its posting', async () => {
		const { store, get } = setup();
		await store.load();
		store.handle(
			runUpdated({
				workflow: 'approve_packet',
				input: { posting_id: 10, packet_id: 3 }
			})
		);
		await settle();
		expect(get).toHaveBeenCalledTimes(1);
		store.handle(
			runUpdated({
				workflow: 'approve_packet',
				input: { posting_id: 9, packet_id: 3 }
			})
		);
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		store.handle(
			runUpdated({
				workflow: 'apply_packet',
				input: { posting_id: 9, packet_id: 3 }
			})
		);
		await settle();
		expect(get).toHaveBeenCalledTimes(3);
	});

	it('drops a response older than the latest load', async () => {
		const first = deferred<Result<PostingPackets>>();
		const get = vi.fn<(c: unknown, id: number) => Promise<Result<PostingPackets>>>();
		get.mockReturnValueOnce(first.promise);
		const newer = makePostingPackets({ run: makeFitRun('running', 61) });
		get.mockResolvedValueOnce({ kind: 'ok', data: newer });
		const store = new PostingPacketsStore(9, { client, get });
		const stale = store.load();
		await store.load();
		first.resolve({ kind: 'ok', data: empty });
		await stale;
		expect(store.state.kind === 'loaded' && store.state.view.kind).toBe('preparing');
	});
});

describe('PacketStore', () => {
	it.each([
		[{ kind: 'ok', data: makePacket() }, 'loaded'],
		[{ kind: 'not-found' }, 'not-found'],
		[{ kind: 'failed' }, 'error']
	] as [Result<ApplicationPacket>, string][])('maps %o to %s', async (result, kind) => {
		const get = vi.fn(async () => result);
		const store = new PacketStore(3, { client, get });
		expect(store.state.kind).toBe('loading');
		await store.load();
		expect(get).toHaveBeenCalledWith(client, 3);
		expect(store.state.kind).toBe(kind);
	});

	it('reloads on run.updated for its own run and on reconnects', async () => {
		const get = vi.fn(async (): Promise<Result<ApplicationPacket>> => ({
			kind: 'ok',
			data: makePacket({ run_id: 50 })
		}));
		const store = new PacketStore(3, { client, get });
		store.handle(runUpdated({ id: 50 }));
		await settle();
		expect(get).not.toHaveBeenCalled();
		await store.load();
		store.handle(runUpdated({ id: 51 }));
		await settle();
		expect(get).toHaveBeenCalledTimes(1);
		store.handle(runUpdated({ id: 50, status: 'succeeded' }));
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		store.handle({ type: 'app.connected' });
		store.handle({ type: 'data.deleted' });
		await settle();
		expect(get).toHaveBeenCalledTimes(4);
	});

	it('reloads on run.updated for approve_packet and apply_packet runs naming it', async () => {
		const get = vi.fn(async (): Promise<Result<ApplicationPacket>> => ({
			kind: 'ok',
			data: makePacket()
		}));
		const store = new PacketStore(3, { client, get });
		await store.load();
		store.handle(
			runUpdated({
				id: 61,
				workflow: 'approve_packet',
				input: { packet_id: 4 }
			})
		);
		store.handle(runUpdated({ id: 61, workflow: 'score_fit', input: { packet_id: 3 } }));
		await settle();
		expect(get).toHaveBeenCalledTimes(1);
		store.handle(
			runUpdated({
				id: 61,
				workflow: 'approve_packet',
				input: { posting_id: 9, packet_id: 3 }
			})
		);
		await settle();
		expect(get).toHaveBeenCalledTimes(2);
		store.handle(
			runUpdated({
				id: 62,
				workflow: 'apply_packet',
				input: { posting_id: 9, packet_id: 3 }
			})
		);
		await settle();
		expect(get).toHaveBeenCalledTimes(3);
	});

	/** A loaded store over `packet` whose action routes answer `result`. */
	async function acting(packet: ApplicationPacket, result: Result<never> | { kind: 'ok' }) {
		const get = vi.fn(async (): Promise<Result<ApplicationPacket>> => ({
			kind: 'ok',
			data: packet
		}));
		const answer = vi.fn(async () => result as Result<never>);
		const deps = {
			client,
			get,
			approve: vi.fn(answer),
			reject: vi.fn(answer),
			request: vi.fn(answer),
			apply: vi.fn(answer),
			undo: vi.fn(answer)
		};
		const store = new PacketStore(3, deps);
		await store.load();
		return { store, ...deps };
	}

	const pending = makePacket({ pending_action_id: 80 });

	it('approves the draft with {} and edits with the input, then reloads', async () => {
		const { store, approve, get } = await acting(pending, { kind: 'ok' });
		const sent = store.approve();
		expect(store.busy).toBe('approve');
		await sent;
		expect(approve).toHaveBeenCalledWith(client, 80, undefined);
		expect(store.busy).toBeNull();
		expect(get).toHaveBeenCalledTimes(2);
		const input = { ...approvalInput(pending), resume_version_id: 2 };
		await store.approve(input);
		expect(approve).toHaveBeenLastCalledWith(client, 80, input);
		expect(toasts()).toEqual([
			['success', 'Packet approved'],
			['success', 'Packet approved']
		]);
	});

	it.each([
		['reject', 'reject', 80, 'Packet rejected', pending],
		['requestApproval', 'request', 3, 'Approval requested', makePacket()],
		['markApplied', 'apply', 3, 'Marking applied…', makeApprovedPacket()],
		['undo', 'undo', 70, 'Undone', makeApprovedPacket()]
	] as const)('%s calls its route, toasts and reloads', async (method, dep, id, done, packet) => {
		const store = await acting(packet, { kind: 'ok' });
		await store.store[method]();
		expect(store[dep]).toHaveBeenCalledWith(client, id);
		expect(store.get).toHaveBeenCalledTimes(2);
		expect(toasts()).toEqual([['success', done]]);
	});

	it('reloads on a conflict and toasts a failure', async () => {
		const conflicted = await acting(pending, { kind: 'conflict' });
		await conflicted.store.reject();
		expect(conflicted.get).toHaveBeenCalledTimes(2);
		expect(toasts()).toEqual([['danger', 'This packet changed. It has been reloaded.']]);
		toast.clear();
		const failed = await acting(makeApprovedPacket(), { kind: 'failed' });
		await failed.store.markApplied();
		expect(failed.get).toHaveBeenCalledTimes(1);
		expect(toasts()).toEqual([['danger', "Couldn't mark it applied. Try again."]]);
	});

	it('leaves a 422 to the edit form and sends one action at a time', async () => {
		const invalid: Result<never> = {
			kind: 'invalid',
			errors: [
				{
					field: 'input',
					message: '$.resume_version_id: no resume has this id'
				}
			]
		};
		const { store, approve } = await acting(pending, invalid);
		expect(await store.approve(approvalInput(pending))).toEqual(invalid);
		expect(toasts()).toEqual([]);
		const first = store.approve();
		expect(await store.reject()).toBeNull();
		await first;
		expect(approve).toHaveBeenCalledTimes(2);
	});

	it('loads the resumes for the edit form', async () => {
		const resumes = vi.fn();
		resumes.mockResolvedValueOnce({ kind: 'failed' });
		resumes.mockResolvedValueOnce({ kind: 'ok', data: [] });
		const store = new PacketStore(3, { client, resumes });
		expect(store.resumes).toEqual({ kind: 'idle' });
		const loading = store.loadResumes();
		expect(store.resumes).toEqual({ kind: 'loading' });
		await loading;
		expect(store.resumes).toEqual({ kind: 'error' });
		await store.loadResumes();
		expect(store.resumes).toEqual({ kind: 'loaded', resumes: [] });
	});
});

describe('packet helpers', () => {
	it('labels and tones the new statuses', () => {
		expect([PACKET_STATUS_LABELS.approved, PACKET_STATUS_TONES.approved]).toEqual([
			'Approved',
			'info'
		]);
		expect([PACKET_STATUS_LABELS.applied, PACKET_STATUS_TONES.applied]).toEqual([
			'Applied',
			'success'
		]);
	});

	it('builds the draft approval input', () => {
		const packet = makePacket({ resume_version_id: null });
		packet.answers[2] = { ...packet.answers[2], answer: null, citations: [] };
		expect(approvalInput(packet)).toEqual({
			packet_id: 3,
			resume_version_id: null,
			answers: {
				work_authorization: 'Yes. I am an EU citizen.',
				notice_period: 'Four weeks after an offer.',
				salary_expectation: null,
				why_leaving: 'I want more scope.',
				why_company: 'You build Rust services.'
			}
		});
	});

	it.each([
		['in review', makePacket({ status: 'in_review' }), { kind: 'inReview' }],
		['dismissed', makePacket({ status: 'dismissed' }), { kind: 'dismissed' }],
		[
			'awaiting approval',
			makePacket({ pending_action_id: 80 }),
			{ kind: 'awaitingApproval', actionId: 80 }
		],
		[
			'requesting',
			makePacket({ latest_run: makeFitRun('running', 61) }),
			{ kind: 'requesting', paused: false }
		],
		[
			'requesting, paused',
			makePacket({ latest_run: makeFitRun('paused', 61) }),
			{ kind: 'requesting', paused: true }
		],
		['requestable', makePacket(), { kind: 'requestable', failed: null }],
		[
			'requestable after a failure',
			makePacket({ latest_run: makeFitRun('failed', 61, 'boom') }),
			{ kind: 'requestable', failed: makeFitRun('failed', 61, 'boom') }
		],
		[
			'approved',
			makeApprovedPacket(),
			{ kind: 'approved', resumeDeleted: false, applying: null, failed: null }
		],
		[
			'approved, resume deleted',
			makeApprovedPacket({ approved_resume_version_id: null }),
			{ kind: 'approved', resumeDeleted: true, applying: null, failed: null }
		],
		[
			'approved, applying',
			makeApprovedPacket({ latest_run: makeFitRun('queued', 62) }),
			{
				kind: 'approved',
				resumeDeleted: false,
				applying: { paused: false },
				failed: null
			}
		],
		[
			'approved, apply failed',
			makeApprovedPacket({ latest_run: makeFitRun('failed', 62, 'boom') }),
			{
				kind: 'approved',
				resumeDeleted: false,
				applying: null,
				failed: makeFitRun('failed', 62, 'boom')
			}
		],
		[
			'applied',
			makeApprovedPacket({
				status: 'applied',
				application_id: 12,
				applied_at: '2026-10-23T09:00:00Z'
			}),
			{ kind: 'applied', appliedAt: '2026-10-23T09:00:00Z', applicationId: 12 }
		]
	] as [string, ApplicationPacket, ReturnType<typeof actionState>][])('%s', (_, packet, state) => {
		expect(actionState(packet)).toEqual(state);
	});
});
