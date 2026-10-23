import { afterEach, describe, expect, it, vi } from 'vitest';
import { createApiClient, type Job, type ProfileImportQueued } from '#lib/api';
import type { watchJob } from '#lib/applications/capture.js';
import { toast } from '#lib/components/ui';
import type { JobPhase } from '#lib/dashboard/job.js';
import type { Result } from '#lib/memories/profile.js';
import { ResumeProfileImport } from './profile-import.svelte';

afterEach(() => toast.clear());

type OnUpdate = (phase: JobPhase, job: Job) => void;

function job(status: string, result: unknown = null): Job {
	return {
		id: 9,
		kind: 'resume_profile',
		status,
		attempts: 1,
		max_attempts: 3,
		run_at: '2026-10-22T00:00:00Z',
		last_error: null,
		result: result as Job['result'],
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:01Z',
		finished_at: null
	};
}

function setup(
	queued: Result<ProfileImportQueued> = {
		kind: 'ok',
		data: { job_id: 9, reused: false }
	}
) {
	const client = createApiClient({
		baseUrl: 'http://api.test',
		fetch: vi.fn()
	});
	let settle!: (result: Result<ProfileImportQueued>) => void;
	const importResume = vi.fn(
		() => new Promise<Result<ProfileImportQueued>>((resolve) => (settle = resolve))
	);
	let update: OnUpdate | null = null;
	const stop = vi.fn();
	const watch = vi.fn<typeof watchJob>((_id, _deps, onUpdate) => {
		update = onUpdate;
		return stop;
	});
	const importer = new ResumeProfileImport({
		client,
		importResume,
		watchJob: watch
	});
	return {
		importer,
		importResume,
		watch,
		stop,
		/** Starts an import and lets the API answer with `queued`. */
		async started(id = 7) {
			const running = importer.start(id);
			settle(queued);
			await running;
		},
		update: (phase: JobPhase, value: Job) => {
			if (!update) throw new Error('not watching');
			update(phase, value);
		}
	};
}

const toasts = () => toast.items.map((t) => [t.tone, t.title, t.description]);

describe('ResumeProfileImport', () => {
	it('is idle, then in flight after start until the job succeeds', async () => {
		const s = setup();
		expect(s.importer.busy).toBe(false);
		await s.started();
		expect(s.importResume).toHaveBeenCalledExactlyOnceWith(expect.anything(), 7);
		expect(s.watch).toHaveBeenCalledOnce();
		expect(s.watch.mock.calls[0][0]).toBe(9);
		expect(s.importer.busy).toBe(true);

		s.update('running', job('running'));
		expect(s.importer.busy).toBe(true);
		expect(toast.items).toEqual([]);

		s.update('succeeded', job('succeeded', { added: 5, already_known: 2, skipped: 0 }));
		expect(s.importer.busy).toBe(false);
		expect(toasts()).toEqual([
			[
				'success',
				'Added to your profile',
				'Added 5 memories, 2 already known. Review them on the Memory page.'
			]
		]);
	});

	it('drops the description when the result is not a summary', async () => {
		const s = setup();
		await s.started();
		s.update('succeeded', job('succeeded', { word: 'pong' }));
		expect(toasts()).toEqual([['success', 'Added to your profile', undefined]]);
	});

	it('says the profile already has a reused import', async () => {
		const s = setup({ kind: 'ok', data: { job_id: 9, reused: true } });
		await s.started();
		s.update('succeeded', job('succeeded', { added: 5, already_known: 0, skipped: 0 }));
		expect(toasts()).toEqual([['success', 'Already in your profile', undefined]]);
		expect(s.importer.busy).toBe(false);
	});

	it('toasts a failed job', async () => {
		const s = setup();
		await s.started();
		s.update('failed', job('failed'));
		expect(toasts()).toEqual([['danger', "Couldn't read this resume", undefined]]);
		expect(s.importer.busy).toBe(false);
	});

	it('toasts a refused request without watching', async () => {
		const s = setup({ kind: 'failed' });
		await s.started();
		expect(s.watch).not.toHaveBeenCalled();
		expect(toasts()).toEqual([['danger', "Couldn't read this resume", undefined]]);
		expect(s.importer.busy).toBe(false);
	});

	it('ignores a second start while in flight', async () => {
		const s = setup();
		await s.started();
		await s.importer.start(7);
		expect(s.importResume).toHaveBeenCalledOnce();
		expect(s.watch).toHaveBeenCalledOnce();
	});

	it('stops watching on destroy', async () => {
		const s = setup();
		await s.started();
		s.importer.destroy();
		expect(s.stop).toHaveBeenCalledOnce();
	});
});
