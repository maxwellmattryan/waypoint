/** API origin used when `VITE_API_URL` is not set. */
export const DEFAULT_API_URL = 'http://127.0.0.1:8000';

/** Path prefix the Vite dev server proxies to the API, with the prefix stripped. */
export const API_PREFIX = '/api';

/**
 * Where the dev server's `/api` proxy sends requests: `VITE_API_URL`, or the local API.
 * Used only by `vite.config.ts`; the browser never calls the API origin directly. Takes the
 * env as a parameter so tests can pass their own values instead of stubbing `import.meta.env`.
 */
export function apiTarget(env: Record<string, string | undefined> = import.meta.env): string {
	return env.VITE_API_URL || DEFAULT_API_URL;
}

/**
 * Base URL for API calls from the browser: the page's own origin plus {@link API_PREFIX}.
 * Absolute because Node's `Request` (used in tests) rejects relative URLs.
 */
export function apiBaseUrl(
	origin: string = globalThis.location?.origin ?? 'http://127.0.0.1:5173'
): string {
	return `${origin}${API_PREFIX}`;
}
