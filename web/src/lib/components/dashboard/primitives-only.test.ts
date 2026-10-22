import { describe, expect, it } from 'vitest';

// AGENTS.md invariants for screens: interactive elements come from the primitives (which bake in
// cursor and state styles), and spacing stays on the 4px grid. Covers the app shell, the dashboard
// cards, the applications and resumes components and pages, and the root layout and page.
const sources = import.meta.glob(
	[
		'../app/*.svelte',
		'./*.svelte',
		'../applications/*.svelte',
		'../resumes/*.svelte',
		'/src/routes/+layout.svelte',
		'/src/routes/+page.svelte',
		'/src/routes/applications/**/*.svelte',
		'/src/routes/resumes/**/*.svelte'
	],
	{ query: '?raw', import: 'default', eager: true }
) as Record<string, string>;

// Mirrors the half-step regex in web/src/lib/components/ui/grid.test.ts.
const halfStep = /(?<![\w-])-?[a-z]+(?:-[a-z]+)*-[0-3]\.5(?![\d])/g;
const rawInteractive = /<(button|a|input|select|textarea|dialog)[\s>]/g;
const cursor = /\bcursor-/g;

describe('primitives only', () => {
	it('screens use only primitives', () => {
		const files = Object.keys(sources);
		expect(files).toEqual(
			expect.arrayContaining([
				'/src/lib/components/app/AppShell.svelte',
				'/src/lib/components/dashboard/ClaudeCard.svelte',
				'/src/lib/components/dashboard/GoogleCard.svelte',
				'/src/lib/components/dashboard/SystemCard.svelte',
				'/src/lib/components/applications/ApplicationForm.svelte',
				'/src/lib/components/applications/ApplicationFormDialog.svelte',
				'/src/lib/components/applications/ApplicationListItem.svelte',
				'/src/lib/components/applications/StatusBadge.svelte',
				'/src/lib/components/applications/StatusSelect.svelte',
				'/src/routes/+layout.svelte',
				'/src/routes/+page.svelte',
				'/src/routes/applications/+page.svelte',
				'/src/routes/applications/[id]/+page.svelte',
				'/src/lib/components/resumes/ResumeListItem.svelte',
				'/src/lib/components/resumes/ResumeSelect.svelte',
				'/src/lib/components/resumes/ResumeUploadDialog.svelte',
				'/src/routes/resumes/+page.svelte',
				'/src/routes/resumes/[id]/+page.svelte'
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
