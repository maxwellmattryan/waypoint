import type {
	GoalStatus,
	Plan,
	PlanCompanyInput,
	PlanEdit,
	PlanStep,
	PlanStepKind,
	PlanStepStatus,
	Source
} from '#lib/api';
import type { BadgeTone } from '#lib/components/ui';

export const GOAL_STATUS_LABELS: Record<GoalStatus, string> = {
	planning: 'Planning',
	needs_answers: 'Needs answers',
	planned: 'Planned',
	running: 'Running',
	completed: 'Completed',
	failed: 'Failed'
};

export const GOAL_STATUS_TONES: Record<GoalStatus, BadgeTone> = {
	planning: 'info',
	needs_answers: 'warning',
	planned: 'primary',
	running: 'info',
	completed: 'success',
	failed: 'danger'
};

export const STEP_LABELS: Record<PlanStepKind, string> = {
	target_companies: 'Target companies',
	source: 'Source boards',
	score: 'Score fit',
	shortlist: 'Shortlist',
	prep: 'Prepare applications'
};

export const STEP_STATUS_LABELS: Record<PlanStepStatus, string> = {
	pending: 'Pending',
	running: 'Running',
	succeeded: 'Done',
	failed: 'Failed',
	skipped: 'Skipped'
};

export const STEP_STATUS_TONES: Record<PlanStepStatus, BadgeTone> = {
	pending: 'neutral',
	running: 'info',
	succeeded: 'success',
	failed: 'danger',
	skipped: 'neutral'
};

/** How long ago `iso` was, relative to `now`: "just now", "2 min ago", "3 h ago", "4 days ago". */
export function timeAgo(iso: string, now: Date = new Date()): string {
	const seconds = Math.max(0, Math.round((now.getTime() - new Date(iso).getTime()) / 1000));
	if (seconds < 60) return 'just now';
	const minutes = Math.floor(seconds / 60);
	if (minutes < 60) return `${minutes} min ago`;
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return `${hours} h ago`;
	const days = Math.floor(hours / 24);
	return days === 1 ? '1 day ago' : `${days} days ago`;
}

/**
 * The plan's version line: "Version 3 · edited by you · 2 min ago", or "Version 2 · by the
 * planner after the source step failed · just now".
 */
export function versionLine(plan: Plan, now: Date = new Date()): string {
	let who: string;
	switch (plan.trigger) {
		case 'user_edit':
			who = 'edited by you';
			break;
		case 'user_replan':
			who = 'by the planner, replanned at your request';
			break;
		case 'step_failed':
			who = plan.failed_step
				? `by the planner after the ${STEP_LABELS[plan.failed_step].toLowerCase()} step failed`
				: 'by the planner after a step failed';
			break;
		default:
			who = 'by the planner';
	}
	return `Version ${plan.version} · ${who} · ${timeAgo(plan.created_at, now)}`;
}

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json =>
	typeof value === 'object' && value !== null && !Array.isArray(value);

/** A count from a step's output; a missing or odd one reads 0. */
function count(output: Json, key: string): number {
	const value = output[key];
	return typeof value === 'number' ? value : 0;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** One line about a step: its error, its outcome, or where it is. */
export function stepSummary(step: PlanStep): string {
	if (step.status === 'failed') return step.last_error ?? 'Failed';
	if (step.status === 'pending') return 'Waiting for the steps before it';
	const output = isObject(step.output) ? step.output : null;
	if (step.status === 'running' || !output) {
		if (step.kind === 'source') return 'Reading the boards…';
		if (step.kind === 'score') return 'Scoring postings…';
		return 'Running…';
	}
	switch (step.kind) {
		case 'target_companies': {
			const unverified = count(output, 'unverified');
			const companies = plural(count(output, 'companies'), 'company', 'companies');
			return unverified ? `${companies}, ${plural(unverified, 'unverified board')}` : companies;
		}
		case 'source':
			return `${plural(count(output, 'boards'), 'board')} read: ${count(output, 'inserted')} new, ${count(output, 'updated')} updated`;
		case 'score': {
			const parts = [`${count(output, 'queued')} scored`];
			if (count(output, 'reused')) parts.push(`${count(output, 'reused')} already scored`);
			if (count(output, 'in_review')) parts.push(`${count(output, 'in_review')} in review`);
			if (count(output, 'failed')) parts.push(`${count(output, 'failed')} failed`);
			return parts.join(' · ');
		}
		case 'shortlist': {
			const shortlisted = `${count(output, 'shortlisted')} shortlisted`;
			const waiting = count(output, 'in_review');
			return waiting ? `${shortlisted} · ${waiting} waiting in review` : shortlisted;
		}
		case 'prep':
			return typeof output.note === 'string' ? output.note : 'Skipped';
	}
}

/** The editor's copy of one company. */
export interface CompanyDraft {
	name: string;
	source: Source | '';
	board: string;
	reason: string;
}

/** The editor's copy of a plan. */
export interface PlanDraft {
	companies: CompanyDraft[];
	title_keywords: string[];
	locations: string[];
	remote_only: boolean;
	max_to_score: string;
	shortlist_size: string;
	min_score: string;
	prep_notes: string;
}

/** A plan as the editor starts it. */
export function draftOf(plan: Plan): PlanDraft {
	return {
		companies: plan.companies.map(({ name, source, board, reason }) => ({
			name,
			source,
			board,
			reason: reason ?? ''
		})),
		title_keywords: [...plan.title_keywords],
		locations: [...plan.locations],
		remote_only: plan.remote_only,
		max_to_score: String(plan.max_to_score),
		shortlist_size: String(plan.shortlist_size),
		min_score: String(plan.min_score),
		prep_notes: plan.prep_notes
	};
}

/** A number field's value: an integer, or null when blank or not a whole number. */
function integer(text: string): number | null {
	const trimmed = text.trim();
	return /^-?\d+$/.test(trimmed) ? Number(trimmed) : null;
}

/** The `PUT /goals/{id}/plan` body for `draft` on top of plan `base`. */
export function editBody(base: number, draft: PlanDraft): PlanEdit {
	const companies: PlanCompanyInput[] = draft.companies.map((c) => ({
		name: c.name.trim(),
		source: c.source,
		board: c.board.trim(),
		reason: c.reason.trim() || null
	}));
	return {
		base_plan_id: base,
		companies,
		title_keywords: draft.title_keywords,
		locations: draft.locations,
		remote_only: draft.remote_only,
		max_to_score: integer(draft.max_to_score),
		shortlist_size: integer(draft.shortlist_size),
		min_score: integer(draft.min_score),
		prep_notes: draft.prep_notes.trim() || null
	};
}

// The API's rules and messages (api/src/goals/mod.rs, api/src/sourcing/mod.rs).
export const REQUIRED = 'Required';
export const BAD_SOURCE = 'Must be greenhouse, lever or ashby';
export const BAD_SLUG = 'Must be 1–100 letters, digits, hyphens or underscores';
export const DUPLICATE_BOARD = 'Duplicate board';
const SLUG = /^[A-Za-z0-9_-]{1,100}$/;
const SOURCES: readonly string[] = ['greenhouse', 'lever', 'ashby'];

/** One company's field errors, as the route reports them. */
export function companyErrors(
	company: CompanyDraft,
	others: CompanyDraft[] = []
): Partial<Record<'name' | 'source' | 'board', string>> {
	const errors: Partial<Record<'name' | 'source' | 'board', string>> = {};
	const name = company.name.trim();
	if (!name) errors.name = REQUIRED;
	else if ([...name].length > 100) errors.name = 'At most 100 characters';
	if (!company.source) errors.source = REQUIRED;
	else if (!SOURCES.includes(company.source)) errors.source = BAD_SOURCE;
	const board = company.board.trim();
	if (!board) errors.board = REQUIRED;
	else if (!SLUG.test(board)) errors.board = BAD_SLUG;
	else if (others.some((o) => o.source === company.source && o.board.trim() === board)) {
		errors.board = DUPLICATE_BOARD;
	}
	return errors;
}

/** Every field error of `draft` the editor can tell before sending, keyed like the route's. */
export function draftErrors(draft: PlanDraft): Record<string, string> {
	const errors: Record<string, string> = {};
	if (draft.companies.length === 0) errors.companies = REQUIRED;
	else if (draft.companies.length > 20) errors.companies = 'At most 20 companies';
	draft.companies.forEach((company, index) => {
		const found = companyErrors(company, draft.companies.slice(0, index));
		for (const [field, message] of Object.entries(found)) {
			errors[`companies[${index}].${field}`] = message;
		}
	});
	if (draft.title_keywords.length === 0) errors.title_keywords = REQUIRED;
	const range = (field: 'max_to_score' | 'shortlist_size' | 'min_score', lo: number, hi: number) => {
		const value = integer(draft[field]);
		if (draft[field].trim() === '') errors[field] = REQUIRED;
		else if (value === null || value < lo || value > hi) errors[field] = `Must be ${lo} to ${hi}`;
	};
	range('max_to_score', 1, 50);
	range('shortlist_size', 1, 20);
	range('min_score', 0, 100);
	if ([...draft.prep_notes.trim()].length > 1000) errors.prep_notes = 'At most 1000 characters';
	return errors;
}
