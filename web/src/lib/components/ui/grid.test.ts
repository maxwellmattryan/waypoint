import { describe, expect, it } from 'vitest';

// AGENTS.md 4px-grid invariant: primitives and their helpers use whole Tailwind spacing steps only.
const sources = import.meta.glob(['./*.svelte', './*.ts', '!./*.test.ts', '!./index.ts'], {
	query: '?raw',
	import: 'default',
	eager: true
}) as Record<string, string>;

const halfStep = /(?<![\w-])-?[a-z]+(?:-[a-z]+)*-[0-3]\.5(?![\d])/g;

describe('4px grid', () => {
	it('primitives use only whole spacing steps', () => {
		const files = Object.keys(sources);
		expect(files.length).toBeGreaterThan(20);
		const offenders = files.flatMap((file) =>
			[...sources[file].matchAll(halfStep)].map((m) => `${file}: ${m[0]}`)
		);
		expect(offenders).toEqual([]);
	});
});
