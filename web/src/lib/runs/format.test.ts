import { describe, expect, it } from 'vitest';
import {
	formatCost,
	formatDuration,
	formatElapsed,
	formatMs,
	formatTokenPair,
	formatTokens
} from './format';

describe('formatCost', () => {
	it.each([
		[0, '$0.00'],
		[0.0001, '<$0.01'],
		[0.0099, '<$0.01'],
		[0.01, '$0.01'],
		[0.125, '$0.13'],
		[1.5, '$1.50'],
		[12.345, '$12.35']
	])('%d is %s', (usd, want) => {
		expect(formatCost(usd)).toBe(want);
	});
});

describe('formatTokens', () => {
	it.each([
		[0, '0'],
		[999, '999'],
		[12345, '12,345'],
		[1234567, '1,234,567']
	])('%d is %s', (count, want) => {
		expect(formatTokens(count)).toBe(want);
	});

	it('pairs input and output', () => {
		expect(formatTokenPair(12345, 67)).toBe('12,345 in · 67 out');
	});
});

describe('formatMs', () => {
	it.each([
		[null, '—'],
		[0, '0 ms'],
		[850, '850 ms'],
		[999, '999 ms'],
		[1000, '1.0 s'],
		[12_300, '12.3 s'],
		[59_949, '59.9 s'],
		[60_000, '1m 00s'],
		[245_000, '4m 05s'],
		[-5, '0 ms']
	])('%s ms is %s', (ms, want) => {
		expect(formatMs(ms)).toBe(want);
	});
});

describe('formatDuration', () => {
	const start = '2026-10-22T10:00:00Z';
	const now = Date.parse('2026-10-22T10:00:12.300Z');

	it.each([
		[null, null, '—'],
		[start, '2026-10-22T10:00:00.850Z', '850 ms'],
		[start, '2026-10-22T10:04:05Z', '4m 05s'],
		[start, null, '12.3 s']
	])('%s to %s is %s', (from, to, want) => {
		expect(formatDuration(from, to, now)).toBe(want);
	});
});

describe('formatElapsed', () => {
	const started_at = '2026-10-22T10:00:00Z';
	const now = Date.parse('2026-10-22T10:00:12.300Z');

	it.each([
		['running', started_at, null, '12.3 s'],
		['running', started_at, '2026-10-22T10:00:01Z', '12.3 s'],
		['succeeded', started_at, '2026-10-22T10:00:00.850Z', '850 ms'],
		['failed', started_at, '2026-10-22T10:04:05Z', '4m 05s'],
		['queued', started_at, null, '—'],
		['pending', null, null, '—']
	])('%s from %s to %s is %s', (status, start, finished_at, want) => {
		expect(formatElapsed({ status, started_at: start, finished_at }, now)).toBe(want);
	});
});
