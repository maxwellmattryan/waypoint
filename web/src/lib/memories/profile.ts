import type { ApiClient, ProfileImportQueued, ProfileImportSummary } from '#lib/api';
import { run, type Result } from '#lib/applications/api.js';

export type { Result };

/** The largest LinkedIn export the API accepts: 20 MiB, as `MAX_ZIP_BYTES` in api/src/profile/linkedin.rs. */
export const MAX_ZIP_BYTES = 20 * 1024 * 1024;

/** The message for an export over {@link MAX_ZIP_BYTES}, from the client check or a 413. */
export const ZIP_TOO_LARGE = 'Must be 20 MiB or smaller';

/**
 * Imports a LinkedIn data export. The body is the raw file, not multipart, so the API's
 * same-origin guard applies. A 413 is an `invalid` result on the `file` field.
 */
export function importLinkedIn(
	client: ApiClient,
	file: File
): Promise<Result<ProfileImportSummary>> {
	return run(
		200,
		() =>
			client.POST('/profile/linkedin', {
				params: { query: { filename: file.name } },
				// The generated type says `string` (binary); the raw File goes through untouched.
				body: file as unknown as string,
				bodySerializer: (body) => body,
				headers: { 'Content-Type': 'application/zip' }
			}),
		(status) =>
			status === 413
				? {
						kind: 'invalid',
						errors: [{ field: 'file', message: ZIP_TOO_LARGE }]
					}
				: undefined
	);
}

/** Queues Claude to read resume version `id` into proposed memories, or reuses an earlier import. */
export function importResume(client: ApiClient, id: number): Promise<Result<ProfileImportQueued>> {
	return run(202, () => client.POST('/resumes/{id}/profile', { params: { path: { id } } }));
}

/** "Added 12 memories, 3 already known", "Added 1 memory", "Added 0 memories, 2 skipped". */
export function summaryText({ added, already_known, skipped }: ProfileImportSummary): string {
	let text = `Added ${added} ${added === 1 ? 'memory' : 'memories'}`;
	if (already_known > 0) text += `, ${already_known} already known`;
	if (skipped > 0) text += `, ${skipped} skipped`;
	return text;
}

const count = (value: unknown) =>
	typeof value === 'number' && Number.isInteger(value) && value >= 0;

/** Whether a job's `result` has the {@link ProfileImportSummary} shape. */
export function isSummary(value: unknown): value is ProfileImportSummary {
	if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
	const summary = value as Record<string, unknown>;
	return count(summary.added) && count(summary.already_known) && count(summary.skipped);
}
