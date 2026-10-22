import type { Event, Job } from '#lib/api';

/** Where a tracked job is, as the dashboard shows it. */
export type JobPhase = 'queued' | 'running' | 'retrying' | 'deferred' | 'succeeded' | 'failed';

/** Whether a job in this phase may still change. */
export function inFlight(phase: JobPhase): boolean {
	return phase !== 'succeeded' && phase !== 'failed';
}

/** The phase a `job.*` event announces. */
export function phaseOfEvent(type: Extract<Event, { job: Job }>['type']): JobPhase {
	switch (type) {
		case 'job.queued':
			return 'queued';
		case 'job.started':
			return 'running';
		case 'job.retrying':
			return 'retrying';
		case 'job.deferred':
			return 'deferred';
		case 'job.succeeded':
			return 'succeeded';
		case 'job.failed':
			return 'failed';
	}
}

/** The phase of a fetched job: a queued job that already ran is waiting to retry. */
export function phaseOfJob(job: Job): JobPhase {
	switch (job.status) {
		case 'running':
			return 'running';
		case 'succeeded':
			return 'succeeded';
		case 'failed':
			return 'failed';
		default:
			return job.attempts > 0 ? 'retrying' : 'queued';
	}
}

/** The later of two snapshots of one job by `updated_at`; on a tie, `a`. */
export function newerJob(a: Job, b: Job): Job {
	return Date.parse(b.updated_at) > Date.parse(a.updated_at) ? b : a;
}

/** The reply text of a succeeded `test_prompt` job, if it has one. */
export function resultText(job: Job): string | null {
	const result = job.result;
	if (result && typeof result === 'object' && 'text' in result && typeof result.text === 'string') {
		return result.text;
	}
	return null;
}

const SCOPE_LABELS: Record<string, string> = {
	'https://www.googleapis.com/auth/gmail.readonly': 'Gmail (read-only)',
	'https://www.googleapis.com/auth/calendar.events': 'Calendar events'
};

/** A readable name for a granted Google scope; unknown scopes come back verbatim. */
export function scopeLabel(scope: string): string {
	return SCOPE_LABELS[scope] ?? scope;
}
