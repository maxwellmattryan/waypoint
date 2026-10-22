import type {
	ApiClient,
	Application,
	ApplicationInput,
	ApplicationStatus,
	FieldError,
	NewApplication
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

export function setApplicationStatus(
	client: ApiClient,
	id: number,
	status: ApplicationStatus
): Promise<Result<Application>> {
	return run(200, () => client.PUT('/applications/{id}/status', { ...path(id), body: { status } }));
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
