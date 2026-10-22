import type {
	ApiClient,
	Application,
	ApplicationEvent,
	ApplicationInput,
	ApplicationStatus,
	EntryKind,
	FieldError,
	NewApplication,
	Stage
} from '#lib/api';

/** What an applications request came to. Never throws: a network error is `failed`. */
export type Result<T> =
	| { kind: 'ok'; data: T }
	| { kind: 'invalid'; errors: FieldError[] }
	| { kind: 'not-found' }
	| { kind: 'failed' };

interface Sent<T> {
	data?: T;
	error?: unknown;
	response: Response;
}

function fieldErrors(error: unknown): FieldError[] | null {
	if (typeof error !== 'object' || error === null || !('errors' in error)) return null;
	return Array.isArray(error.errors) ? (error.errors as FieldError[]) : null;
}

/** Runs one request and maps its status: `ok` (the given status), 422, 404 or anything else. */
async function run<T>(ok: number, send: () => Promise<Sent<T>>): Promise<Result<T>> {
	try {
		const { data, error, response } = await send();
		if (response.status === ok) return { kind: 'ok', data: data as T };
		if (response.status === 422) {
			const errors = fieldErrors(error);
			if (errors) return { kind: 'invalid', errors };
		}
		if (response.status === 404) return { kind: 'not-found' };
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
