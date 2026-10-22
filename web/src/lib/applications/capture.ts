import {
	connectEvents as realConnectEvents,
	type ApiClient,
	type Capture,
	type CaptureMethod,
	type FieldError,
	type Job
} from '#lib/api';
import { inFlight, newerJob, phaseOfEvent, phaseOfJob, type JobPhase } from '#lib/dashboard/job.js';

/** What `POST /captures` came to. Never throws: a network error is `failed`. */
export type CaptureStart =
	| { kind: 'captured'; capture: Capture }
	| { kind: 'queued'; jobId: number }
	| { kind: 'invalid'; errors: FieldError[] }
	| { kind: 'unreadable'; message: string }
	| { kind: 'failed' };

const METHODS: readonly CaptureMethod[] = ['json_ld', 'greenhouse', 'lever', 'ashby', 'llm'];

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

const nullableString = (value: unknown) => value === null || typeof value === 'string';

/** Whether `value` has the `Capture` shape: a known method and the five fields. */
export function isCapture(value: unknown): value is Capture {
	if (!isObject(value) || !METHODS.includes(value.method as CaptureMethod)) return false;
	const fields = value.fields;
	return (
		isObject(fields) &&
		typeof fields.url === 'string' &&
		(['company', 'title', 'location', 'comp'] as const).every((key) => nullableString(fields[key]))
	);
}

function fieldErrors(error: unknown): FieldError[] | null {
	if (!isObject(error) || !Array.isArray(error.errors)) return null;
	return error.errors as FieldError[];
}

/** Asks the API to read `url`. The URL is sent as typed; the server trims it. */
export async function startCapture(client: ApiClient, url: string): Promise<CaptureStart> {
	try {
		const sent = await client.POST('/captures', { body: { url } });
		// Checked by shape below rather than trusted from the declared types.
		const data: unknown = sent.data;
		const error: unknown = sent.error;
		switch (sent.response.status) {
			case 200:
				return isCapture(data) ? { kind: 'captured', capture: data } : { kind: 'failed' };
			case 202:
				return isObject(data) && typeof data.job_id === 'number'
					? { kind: 'queued', jobId: data.job_id }
					: { kind: 'failed' };
			case 422: {
				const errors = fieldErrors(error);
				return errors ? { kind: 'invalid', errors } : { kind: 'failed' };
			}
			case 502:
				return isObject(error) && typeof error.message === 'string'
					? { kind: 'unreadable', message: error.message }
					: { kind: 'failed' };
			default:
				return { kind: 'failed' };
		}
	} catch {
		return { kind: 'failed' };
	}
}

const BOARD_NAMES: Partial<Record<CaptureMethod, string>> = {
	greenhouse: 'Greenhouse',
	lever: 'Lever',
	ashby: 'Ashby'
};

/** The Add application dialog's description for a capture. */
export function captureDescription(capture: Capture): string {
	const { company, title } = capture.fields;
	if (company === null && title === null) {
		return "Couldn't find the details on that page. Fill them in, then add the application.";
	}
	const check = 'Check the details, then add the application.';
	if (capture.method === 'json_ld') return `Filled from the page's job data. ${check}`;
	if (capture.method === 'llm') return `Filled by Claude from the page text. ${check}`;
	return `Filled from ${BOARD_NAMES[capture.method]}. ${check}`;
}

export interface WatchJobDeps {
	client: ApiClient;
	connectEvents?: typeof realConnectEvents;
}

/**
 * Follows one job over the event stream and reports each newer snapshot with its phase.
 * Events about other jobs are ignored; `app.connected` and `app.lagged` refetch
 * `GET /jobs/{id}`, since the stream has no replay. Stops at `succeeded` or `failed`, or
 * when the returned function is called.
 */
export function watchJob(
	jobId: number,
	{ client, connectEvents = realConnectEvents }: WatchJobDeps,
	onUpdate: (phase: JobPhase, job: Job) => void
): () => void {
	let latest: Job | null = null;
	let stopped = false;
	let close: (() => void) | null = null;

	function stop() {
		if (stopped) return;
		stopped = true;
		close?.();
		close = null;
	}

	function apply(job: Job, phase: JobPhase, fromEvent: boolean) {
		if (stopped || job.id !== jobId) return;
		if (latest) {
			// Of two snapshots the later `updated_at` wins; on a tie an event beats a fetch.
			const winner = fromEvent ? newerJob(job, latest) : newerJob(latest, job);
			if (winner !== job) return;
		}
		latest = job;
		onUpdate(phase, job);
		if (!inFlight(phase)) stop();
	}

	async function refresh() {
		try {
			const { data } = await client.GET('/jobs/{id}', {
				params: { path: { id: jobId } }
			});
			if (data) apply(data, phaseOfJob(data), false);
		} catch {
			// The stream reconnects and refetches; nothing to report meanwhile.
		}
	}

	close = connectEvents({
		event: (event) => {
			switch (event.type) {
				case 'app.connected':
				case 'app.lagged':
					void refresh();
					return;
				case 'queue.paused':
				case 'queue.resumed':
				case 'application.created':
				case 'application.updated':
				case 'application.deleted':
					return;
				default:
					apply(event.job, phaseOfEvent(event.type), true);
			}
		},
		error: () => {}
	});
	if (stopped) close();
	return stop;
}
