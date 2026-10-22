import type { ApiClient, ResumeVersion, ResumeVersionDetail } from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';
import { apiBaseUrl } from '#lib/config.js';
import { TOO_LARGE } from './format';

export type { Result };

const path = (id: number) => ({ params: { path: { id } } });

export function listResumes(client: ApiClient): Promise<Result<ResumeVersion[]>> {
	return run(200, () => client.GET('/resumes'));
}

export function getResume(client: ApiClient, id: number): Promise<Result<ResumeVersionDetail>> {
	return run(200, () => client.GET('/resumes/{id}', path(id)));
}

/**
 * Uploads `file` as a new version named `name`. The body is the raw file, not multipart, so
 * the API's same-origin guard applies. A 413 is an `invalid` result on the `file` field.
 */
export function uploadResume(
	client: ApiClient,
	file: File,
	name: string
): Promise<Result<ResumeVersion>> {
	return run(
		201,
		() =>
			client.POST('/resumes', {
				params: { query: { name, filename: file.name } },
				// The generated type says `string` (binary); the raw File goes through untouched.
				body: file as unknown as string,
				bodySerializer: (body) => body,
				headers: { 'Content-Type': 'application/pdf' }
			}),
		(status) =>
			status === 413 ? { kind: 'invalid', errors: [{ field: 'file', message: TOO_LARGE }] } : undefined
	);
}

export function deleteResume(client: ApiClient, id: number): Promise<Result<void>> {
	return run<void>(204, () => client.DELETE('/resumes/{id}', path(id)));
}

/** Where the browser opens a version's PDF: the API's `/resumes/{id}/pdf` through the proxy. */
export function pdfUrl(id: number, base: string = apiBaseUrl()): string {
	return `${base}/resumes/${id}/pdf`;
}
