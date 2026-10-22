import { cubicIn, cubicOut } from 'svelte/easing';
import { prefersReducedMotion } from 'svelte/motion';

// Motion tokens for Svelte transitions and animations.
// Keep these in sync with the --wp-duration-* and --ease-* tokens in src/lib/styles/tokens.css.

/** Durations in milliseconds. */
export const durations = { fast: 120, base: 200, slow: 320 } as const;

/** Easing functions approximating the CSS `--ease-*` curves. */
export const easings = {
	/** ~ cubic-bezier(0.2, 0, 0, 1) */
	standard: cubicOut,
	/** ~ cubic-bezier(0, 0, 0.2, 1) */
	enter: cubicOut,
	/** ~ cubic-bezier(0.4, 0, 1, 1) */
	exit: cubicIn
} as const;

export type DurationName = keyof typeof durations;

/**
 * The duration in ms for a motion token, or 0 when motion is reduced.
 * `reduced` defaults to the user's `prefers-reduced-motion` setting.
 */
export function motionDuration(
	name: DurationName,
	reduced: boolean = prefersReducedMotion.current
): number {
	return reduced ? 0 : durations[name];
}
