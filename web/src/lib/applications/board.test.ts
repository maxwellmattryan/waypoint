import { describe, expect, it } from 'vitest';
import type { Application } from '#lib/api';
import { durations, easings } from '#lib/motion.js';
import { boardMotion, compareApplications } from './board';

function application(id: number, updated_at: string): Application {
	return {
		id,
		company: 'Acme',
		title: 'Engineer',
		url: null,
		location: null,
		comp: null,
		source: null,
		notes: null,
		status: 'sourced',
		status_changed_at: updated_at,
		created_at: updated_at,
		updated_at
	};
}

describe('board', () => {
	it('compareApplications sorts newest updated_at first, then id descending', () => {
		const rows = [
			application(1, '2026-10-22T00:00:00Z'),
			application(2, '2026-10-22T00:00:05Z'),
			application(3, '2026-10-22T00:00:00Z'),
			// The same instant written with a fraction and an offset still ties.
			application(4, '2026-10-22T02:00:00.000+02:00')
		];
		expect(rows.toSorted(compareApplications).map((row) => row.id)).toEqual([2, 4, 3, 1]);
	});

	it('boardMotion follows reduced motion', () => {
		expect(boardMotion(false)).toEqual({ duration: durations.base, easing: easings.standard });
		expect(boardMotion(true)).toEqual({ duration: 0, easing: easings.standard });
		// No reduced-motion preference in jsdom: the default allows motion.
		expect(boardMotion().duration).toBe(durations.base);
	});
});
