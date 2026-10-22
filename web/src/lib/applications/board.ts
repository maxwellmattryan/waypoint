import { prefersReducedMotion } from 'svelte/motion';
import type { Application } from '#lib/api';
import { easings, motionDuration } from '#lib/motion.js';

/** The list and board order: newest `updated_at` first, ties by `id` descending. */
export function compareApplications(a: Application, b: Application): number {
	return Date.parse(b.updated_at) - Date.parse(a.updated_at) || b.id - a.id;
}

/**
 * The timing of a card moving between or within columns: the `base` motion token with the
 * standard easing, or no motion at all when `reduced`.
 */
export function boardMotion(reduced: boolean = prefersReducedMotion.current): {
	duration: number;
	easing: (t: number) => number;
} {
	return { duration: motionDuration('base', reduced), easing: easings.standard };
}
