import { describe, expect, it } from 'vitest';

// AGENTS.md invariants for screens: interactive elements come from the primitives (which bake in
// cursor and state styles), and spacing stays on the 4px grid. Covers the app shell, the dashboard
// cards and the root layout and page.
const sources = import.meta.glob(
	['../app/*.svelte', './*.svelte', '/src/routes/+layout.svelte', '/src/routes/+page.svelte'],
	{ query: '?raw', import: 'default', eager: true }
) as Record<string, string>;

// Mirrors the half-step regex in web/src/lib/components/ui/grid.test.ts.
const halfStep = /(?<![\w-])-?[a-z]+(?:-[a-z]+)*-[0-3]\.5(?![\d])/g;
const rawInteractive = /<(button|a|input|select|textarea|dialog)[\s>]/g;
const cursor = /\bcursor-/g;

describe('primitives only', () => {
	it('dashboard uses only primitives', () => {
		const files = Object.keys(sources);
		expect(files).toEqual(
			expect.arrayContaining([
				'/src/lib/components/app/AppShell.svelte',
				'/src/lib/components/dashboard/ClaudeCard.svelte',
				'/src/lib/components/dashboard/GoogleCard.svelte',
				'/src/lib/components/dashboard/SystemCard.svelte',
				'/src/routes/+layout.svelte',
				'/src/routes/+page.svelte'
			])
		);
		const offenders = files.flatMap((file) =>
			[rawInteractive, cursor, halfStep].flatMap((pattern) =>
				[...sources[file].matchAll(pattern)].map((m) => `${file}: ${m[0]}`)
			)
		);
		expect(offenders).toEqual([]);
	});
});
