import type {
	ApiClient,
	Application,
	ApplicationEvent,
	ApplicationInput,
	ApplicationStatus,
	EntryKind,
	FieldError,
	NewApplication,
	Run,
	Stage,
	TidyRequest
} from '#lib/api';

/**
 * What an API request came to. Never throws: a network error is `failed`. `conflict` (409) is
 * a change the API refused because something references the row, e.g. deleting a linked resume.
 */
export type Result<T> =
	| { kind: 'ok'; data: T }
	| { kind: 'invalid'; errors: FieldError[] }
	| { kind: 'not-found' }
	| { kind: 'conflict' }
	| { kind: 'failed' };

export interface Sent<T> {
	data?: T;
	error?: unknown;
	response: Response;
}

function fieldErrors(error: unknown): FieldError[] | null {
	if (typeof error !== 'object' || error === null || !('errors' in error)) return null;
	return Array.isArray(error.errors) ? (error.errors as FieldError[]) : null;
}

/**
 * Runs one request and maps its status: `ok` (the given status), 422, 404, 409 or anything
 * else. `extra` maps a route's own statuses first, e.g. 413 on an upload.
 */
export async function run<T>(
	ok: number,
	send: () => Promise<Sent<T>>,
	extra?: (status: number) => Result<T> | undefined
): Promise<Result<T>> {
	try {
		const { data, error, response } = await send();
		if (response.status === ok) return { kind: 'ok', data: data as T };
		const mapped = extra?.(response.status);
		if (mapped) return mapped;
		if (response.status === 422) {
			const errors = fieldErrors(error);
			if (errors) return { kind: 'invalid', errors };
		}
		if (response.status === 404) return { kind: 'not-found' };
		if (response.status === 409) return { kind: 'conflict' };
		return { kind: 'failed' };
	} catch {
		return { kind: 'failed' };
	}
}

const path = (id: number) => ({ params: { path: { id } } });

export function listApplications(client: ApiClient): Promise<Result<Application[]>> {
	return run(200, () => client.GET('/applications'));
}

export function getApplication(client: ApiClient, id: number): Promise<Result<Application>> {
	return run(200, () => client.GET('/applications/{id}', path(id)));
}

export function createApplication(
	client: ApiClient,
	body: NewApplication
): Promise<Result<Application>> {
	return run(201, () => client.POST('/applications', { body }));
}

export function updateApplication(
	client: ApiClient,
	id: number,
	body: ApplicationInput
): Promise<Result<Application>> {
	return run(200, () => client.PUT('/applications/{id}', { ...path(id), body }));
}

/** Sets the status; `feedback` (only with `rejected`) is sent only when given. */
export function setApplicationStatus(
	client: ApiClient,
	id: number,
	status: ApplicationStatus,
	feedback?: string
): Promise<Result<Application>> {
	const body = feedback === undefined ? { status } : { status, feedback };
	return run(200, () => client.PUT('/applications/{id}/status', { ...path(id), body }));
}

/** Sets the resume version sent with an application, or clears it with `null`. */
export function setApplicationResume(
	client: ApiClient,
	id: number,
	resumeVersionId: number | null
): Promise<Result<Application>> {
	return run(200, () =>
		client.PUT('/applications/{id}/resume', {
			...path(id),
			body: { resume_version_id: resumeVersionId }
		})
	);
}

export function deleteApplication(client: ApiClient, id: number): Promise<Result<void>> {
	return run<void>(204, () => client.DELETE('/applications/{id}', path(id)));
}

const stagePath = (id: number, stage_id: number) => ({
	params: { path: { id, stage_id } }
});

export function listStages(client: ApiClient, id: number): Promise<Result<Stage[]>> {
	return run(200, () => client.GET('/applications/{id}/stages', path(id)));
}

/** Appends a stage; resolves to the whole ordered list. */
export function addStage(client: ApiClient, id: number, name: string): Promise<Result<Stage[]>> {
	return run(201, () => client.POST('/applications/{id}/stages', { ...path(id), body: { name } }));
}

/** Completes or reopens a stage; resolves to the whole ordered list. */
export function setStageCompleted(
	client: ApiClient,
	id: number,
	stageId: number,
	completed: boolean
): Promise<Result<Stage[]>> {
	return run(200, () =>
		client.PUT('/applications/{id}/stages/{stage_id}', {
			...stagePath(id, stageId),
			body: { completed }
		})
	);
}

/** Puts every stage in the given order; resolves to the whole ordered list. */
export function reorderStages(
	client: ApiClient,
	id: number,
	stageIds: number[]
): Promise<Result<Stage[]>> {
	return run(200, () =>
		client.PUT('/applications/{id}/stage-order', {
			...path(id),
			body: { stage_ids: stageIds }
		})
	);
}

/** Removes a stage; resolves to the remaining ordered list. */
export function removeStage(
	client: ApiClient,
	id: number,
	stageId: number
): Promise<Result<Stage[]>> {
	return run(200, () =>
		client.DELETE('/applications/{id}/stages/{stage_id}', stagePath(id, stageId))
	);
}

/** The application's timeline, newest first. */
export function listEvents(client: ApiClient, id: number): Promise<Result<ApplicationEvent[]>> {
	return run(200, () => client.GET('/applications/{id}/events', path(id)));
}

/** Adds a note, or rejection feedback for a rejected application, to the timeline. */
export function addEntry(
	client: ApiClient,
	id: number,
	kind: EntryKind,
	body: string
): Promise<Result<ApplicationEvent>> {
	return run(201, () =>
		client.POST('/applications/{id}/events', {
			...path(id),
			body: { kind, body }
		})
	);
}

/**
 * Starts the `tidy_application` sample workflow for an application; resolves to the queued
 * run. A 422 maps to `note`, `message` or `confidence`.
 */
export function startTidy(
	client: ApiClient,
	id: number,
	body: TidyRequest
): Promise<Result<Run>> {
	return run(201, () => client.POST('/applications/{id}/tidy', { ...path(id), body }));
}
