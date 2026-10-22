import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/svelte';
import { afterEach } from 'vitest';

// Unmount components between tests (auto-cleanup needs Vitest globals, which this repo doesn't use).
afterEach(() => cleanup());

// jsdom has no matchMedia, and svelte/motion's prefersReducedMotion reads it.
if (typeof window !== 'undefined' && !window.matchMedia) {
	Object.defineProperty(window, 'matchMedia', {
		writable: true,
		value: (query: string): MediaQueryList => ({
			matches: false,
			media: query,
			onchange: null,
			addListener: () => {},
			removeListener: () => {},
			addEventListener: () => {},
			removeEventListener: () => {},
			dispatchEvent: () => false
		})
	});
}

// jsdom has no Web Animations API, and Svelte transitions call `element.animate`. This stub
// finishes every animation on the next microtask so intros and outros complete.
if (typeof Element !== 'undefined' && !Element.prototype.animate) {
	Element.prototype.animate = function () {
		const animation = {
			onfinish: null as ((event: unknown) => void) | null,
			oncancel: null,
			currentTime: 0,
			playState: 'running',
			finished: Promise.resolve(),
			cancel: () => {},
			finish: () => {},
			play: () => {},
			pause: () => {},
			reverse: () => {},
			addEventListener: () => {},
			removeEventListener: () => {}
		};
		queueMicrotask(() => {
			animation.playState = 'finished';
			animation.onfinish?.({});
		});
		return animation as unknown as Animation;
	};
}

// `animate:flip` reads running animations before measuring.
if (typeof Element !== 'undefined' && !Element.prototype.getAnimations) {
	Element.prototype.getAnimations = () => [];
}
