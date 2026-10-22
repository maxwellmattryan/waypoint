const tokens = new Intl.NumberFormat();

/** A cost in US dollars: `$0.00` for nothing, `<$0.01` under a cent, otherwise `$X.XX`. */
export function formatCost(usd: number): string {
	if (usd === 0) return '$0.00';
	if (usd < 0.01) return '<$0.01';
	return `$${usd.toFixed(2)}`;
}

/** A token count with the locale's grouping, e.g. `12,345`. */
export function formatTokens(count: number): string {
	return tokens.format(count);
}

/** Input and output tokens as `<in> in · <out> out`. */
export function formatTokenPair(input: number, output: number): string {
	return `${formatTokens(input)} in · ${formatTokens(output)} out`;
}

/** Milliseconds as `850 ms`, `12.3 s` or `4m 05s`; `—` when unknown. */
export function formatMs(ms: number | null): string {
	if (ms === null || !Number.isFinite(ms)) return '—';
	const whole = Math.max(0, Math.round(ms));
	if (whole < 1000) return `${whole} ms`;
	if (whole < 60_000) return `${(whole / 1000).toFixed(1)} s`;
	const seconds = Math.floor(whole / 1000);
	return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`;
}

/**
 * The time from `startIso` to `endIso`, or to `now` while there is no end, formatted like
 * {@link formatMs}. A missing start is `—`.
 */
export function formatDuration(
	startIso: string | null,
	endIso: string | null,
	now: number = Date.now()
): string {
	if (startIso === null) return '—';
	const end = endIso === null ? now : Date.parse(endIso);
	return formatMs(end - Date.parse(startIso));
}

/** The parts of a run or step that say how long it took. */
export interface Timed {
	status: string;
	started_at: string | null;
	finished_at: string | null;
}

/**
 * How long a run or step took: up to `now` while `running`, up to `finished_at` once it has
 * one, and `—` otherwise (not started, or waiting to run again).
 */
export function formatElapsed(
	{ status, started_at, finished_at }: Timed,
	now: number = Date.now()
): string {
	if (status === 'running') return formatDuration(started_at, null, now);
	if (finished_at === null) return '—';
	return formatDuration(started_at, finished_at, now);
}
