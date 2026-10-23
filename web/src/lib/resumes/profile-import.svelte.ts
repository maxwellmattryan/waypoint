import type { ApiClient } from '#lib/api';
import { watchJob as realWatchJob } from '#lib/applications/capture.js';
import { toast } from '#lib/components/ui';
import { importResume as realImportResume, isSummary, summaryText } from '#lib/memories/profile.js';

export interface ResumeProfileImportDeps {
	client: ApiClient;
	importResume?: typeof realImportResume;
	watchJob?: typeof realWatchJob;
}

const FAILED = "Couldn't read this resume";

/**
 * The resume page's **Add to profile** action: queues the `resume_profile` job, follows it
 * over the event stream and toasts how it ended. One import at a time; the job keeps
 * running on the server if the page goes away, only the toast is lost.
 */
export class ResumeProfileImport {
	/** Whether an import is being queued or its job is still running. */
	busy = $state(false);

	#client: ApiClient;
	#importResume: typeof realImportResume;
	#watchJob: typeof realWatchJob;
	#stop: (() => void) | null = null;
	#destroyed = false;

	constructor({
		client,
		importResume = realImportResume,
		watchJob = realWatchJob
	}: ResumeProfileImportDeps) {
		this.#client = client;
		this.#importResume = importResume;
		this.#watchJob = watchJob;
	}

	/** Imports resume version `id`. Ignored while an import is in flight. */
	async start(id: number): Promise<void> {
		if (this.busy || this.#destroyed) return;
		this.busy = true;
		const result = await this.#importResume(this.#client, id);
		if (this.#destroyed) return;
		if (result.kind !== 'ok') {
			this.busy = false;
			toast.error(FAILED);
			return;
		}
		const { job_id, reused } = result.data;
		this.#stop = this.#watchJob(job_id, { client: this.#client }, (phase, job) => {
			if (phase === 'succeeded') {
				this.#settle();
				if (reused) {
					toast.success('Already in your profile');
				} else {
					toast.success('Added to your profile', {
						description: isSummary(job.result)
							? `${summaryText(job.result)}. Review them on the Memory page.`
							: undefined
					});
				}
			} else if (phase === 'failed') {
				this.#settle();
				toast.error(FAILED);
			}
		});
	}

	/** Stops following the job. */
	destroy(): void {
		this.#destroyed = true;
		this.#stop?.();
		this.#stop = null;
	}

	#settle() {
		this.busy = false;
		this.#stop = null;
	}
}
