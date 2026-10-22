import { apiBaseUrl } from '#lib/config.js';

/**
 * The `202` body of `POST /dev/jobs/test-prompt`. Typed here by hand because the debug-only
 * `/dev` routes are deliberately left out of the OpenAPI contract; every other call goes
 * through the generated client.
 */
interface TestPromptAccepted {
	job_id: number;
}

/** The outcome of {@link startTestPrompt}. `status` is `0` on a network error. */
export type StartTestPromptResult = { ok: true; jobId: number } | { ok: false; status: number };

export interface StartTestPromptOptions {
	fetch?: typeof fetch;
	baseUrl?: string;
}

/** Enqueues a `test_prompt` job (one `claude -p` round trip) on a debug API build. */
export async function startTestPrompt({
	fetch = globalThis.fetch,
	baseUrl = apiBaseUrl()
}: StartTestPromptOptions = {}): Promise<StartTestPromptResult> {
	let response: Response;
	try {
		response = await fetch(`${baseUrl}/dev/jobs/test-prompt`, {
			method: 'POST',
			headers: { 'X-Waypoint-Dev': '1' }
		});
	} catch {
		return { ok: false, status: 0 };
	}
	if (response.status !== 202) return { ok: false, status: response.status };
	try {
		const body = (await response.json()) as Partial<TestPromptAccepted>;
		if (Number.isSafeInteger(body.job_id)) return { ok: true, jobId: body.job_id as number };
	} catch {
		// Falls through: an unreadable body is a failure like any other.
	}
	return { ok: false, status: response.status };
}
