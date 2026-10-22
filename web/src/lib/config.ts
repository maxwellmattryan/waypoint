/** API origin used when `VITE_API_URL` is not set. */
export const DEFAULT_API_URL = 'http://127.0.0.1:8000';

/**
 * Base URL of the Waypoint API. Takes the env as a parameter so tests can pass
 * their own values instead of stubbing `import.meta.env`.
 */
export function apiBaseUrl(
	env: Record<string, string | undefined> = import.meta.env
): string {
	return env.VITE_API_URL || DEFAULT_API_URL;
}
