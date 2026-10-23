import { describe, expect, it } from 'vitest';

// AGENTS.md invariants for screens: interactive elements come from the primitives (which bake in
// cursor and state styles), and spacing stays on the 4px grid. Covers the app shell, the dashboard
// cards, the applications, resumes, memories, runs, review and postings components and pages, and
// the root layout and page.
const sources = import.meta.glob(
	[
		'../app/*.svelte',
		'./*.svelte',
		'../applications/*.svelte',
		'../resumes/*.svelte',
		'../memories/*.svelte',
		'../runs/*.svelte',
		'../review/*.svelte',
		'../postings/*.svelte',
		'/src/routes/+layout.svelte',
		'/src/routes/+page.svelte',
		'/src/routes/applications/**/*.svelte',
		'/src/routes/resumes/**/*.svelte',
		'/src/routes/memories/**/*.svelte',
		'/src/routes/runs/**/*.svelte',
		'/src/routes/review/**/*.svelte',
		'/src/routes/postings/**/*.svelte'
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
				'/src/lib/components/dashboard/AgentsCard.svelte',
				'/src/lib/components/dashboard/ClaudeCard.svelte',
				'/src/lib/components/dashboard/DataCard.svelte',
				'/src/lib/components/dashboard/DeleteAllDataDialog.svelte',
				'/src/lib/components/dashboard/GmailSyncCard.svelte',
				'/src/lib/components/dashboard/GoogleCard.svelte',
				'/src/lib/components/dashboard/SystemCard.svelte',
				'/src/lib/components/applications/ApplicationBoard.svelte',
				'/src/lib/components/applications/ApplicationForm.svelte',
				'/src/lib/components/applications/ApplicationFormDialog.svelte',
				'/src/lib/components/applications/ApplicationListItem.svelte',
				'/src/lib/components/applications/BoardCard.svelte',
				'/src/lib/components/applications/BoardColumn.svelte',
				'/src/lib/components/applications/NoteComposer.svelte',
				'/src/lib/components/applications/RejectionDialog.svelte',
				'/src/lib/components/applications/StageList.svelte',
				'/src/lib/components/applications/StageRow.svelte',
				'/src/lib/components/applications/StatusBadge.svelte',
				'/src/lib/components/applications/StatusSelect.svelte',
				'/src/lib/components/applications/TidyDialog.svelte',
				'/src/lib/components/applications/Timeline.svelte',
				'/src/lib/components/applications/TimelineItem.svelte',
				'/src/routes/+layout.svelte',
				'/src/routes/+page.svelte',
				'/src/routes/applications/+page.svelte',
				'/src/routes/applications/[id]/+page.svelte',
				'/src/lib/components/resumes/ResumeListItem.svelte',
				'/src/lib/components/resumes/ResumeSelect.svelte',
				'/src/lib/components/resumes/ResumeUploadDialog.svelte',
				'/src/routes/resumes/+page.svelte',
				'/src/routes/resumes/[id]/+page.svelte',
				'/src/lib/components/memories/MemoryFilters.svelte',
				'/src/lib/components/memories/MemoryFormDialog.svelte',
				'/src/lib/components/memories/MemoryItem.svelte',
				'/src/lib/components/memories/MemorySection.svelte',
				'/src/routes/memories/+page.svelte',
				'/src/lib/components/runs/MemoriesUsed.svelte',
				'/src/lib/components/runs/PermissionBadge.svelte',
				'/src/lib/components/runs/RunDetailView.svelte',
				'/src/lib/components/runs/RunError.svelte',
				'/src/lib/components/runs/RunListItem.svelte',
				'/src/lib/components/runs/RunStatusBadge.svelte',
				'/src/lib/components/runs/RunSummary.svelte',
				'/src/lib/components/runs/StepItem.svelte',
				'/src/lib/components/runs/StepStatusBadge.svelte',
				'/src/lib/components/runs/ToolCallItem.svelte',
				'/src/lib/components/runs/ToolCallStatusBadge.svelte',
				'/src/routes/runs/+page.svelte',
				'/src/routes/runs/[id]/+page.svelte',
				'/src/lib/components/review/ActionLogItem.svelte',
				'/src/lib/components/review/EditActionDialog.svelte',
				'/src/lib/components/review/ItemMeta.svelte',
				'/src/lib/components/review/PendingActionItem.svelte',
				'/src/lib/components/review/RejectActionDialog.svelte',
				'/src/lib/components/review/ReviewItemCard.svelte',
				'/src/lib/components/review/RunLink.svelte',
				'/src/routes/review/+page.svelte',
				'/src/lib/components/postings/PostingBadges.svelte',
				'/src/lib/components/postings/PostingDetailView.svelte',
				'/src/lib/components/postings/PostingFilters.svelte',
				'/src/lib/components/postings/PostingListItem.svelte',
				'/src/lib/components/postings/SourceCompanyDialog.svelte',
				'/src/routes/postings/+page.svelte',
				'/src/routes/postings/[id]/+page.svelte'
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
