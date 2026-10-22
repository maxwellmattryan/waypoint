import { describe, expect, it } from 'vitest';
import { durations, easings, motionDuration } from './motion';

describe('motion', () => {
	it('motionDuration returns token ms and 0 when reduced', () => {
		expect(motionDuration('fast', false)).toBe(120);
		expect(motionDuration('base', false)).toBe(200);
		expect(motionDuration('slow', false)).toBe(320);
		for (const name of Object.keys(durations) as (keyof typeof durations)[]) {
			expect(motionDuration(name, true)).toBe(0);
		}
	});

	it('defaults to allowing motion when no reduced-motion preference is set', () => {
		expect(motionDuration('base')).toBe(durations.base);
	});

	it('easings map 0 to 0 and 1 to 1', () => {
		for (const ease of Object.values(easings)) {
			expect(ease(0)).toBe(0);
			expect(ease(1)).toBe(1);
		}
	});
});
