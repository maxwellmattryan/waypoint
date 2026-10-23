import { resolve } from '$app/paths';

/** The memories page with memory `id` scrolled to, focused and highlighted. */
export function memoryHref(id: number): string {
	return `${resolve('/memories')}?memory=${id}`;
}

/**
 * The `memory` query parameter as a positive integer, or `null` for anything else (missing,
 * blank, zero, negative, fractional, non-numeric or past `Number.MAX_SAFE_INTEGER`).
 */
export function parseMemoryParam(raw: string | null): number | null {
	if (raw === null || !/^[1-9]\d*$/.test(raw)) return null;
	const id = Number(raw);
	return Number.isSafeInteger(id) ? id : null;
}
