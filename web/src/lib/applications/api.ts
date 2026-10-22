import type {
	ApiClient,
	Application,
	ApplicationInput,
	ApplicationStatus,
	FieldError,
	NewApplication
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

export function setApplicationStatus(
	client: ApiClient,
	id: number,
	status: ApplicationStatus
): Promise<Result<Application>> {
	return run(200, () => client.PUT('/applications/{id}/status', { ...path(id), body: { status } }));
}

export function deleteApplication(client: ApiClient, id: number): Promise<Result<void>> {
	return run<void>(204, () => client.DELETE('/applications/{id}', path(id)));
}
