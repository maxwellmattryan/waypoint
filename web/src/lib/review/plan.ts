import type { ReviewItem, Source } from '#lib/api';
import type { BadgeTone } from '#lib/components/ui';

/** A goal's plan that wasn't made ready: unsure, or with items validation dropped. */
export const PLAN_KIND = 'plan_goal.plan';

export interface PlanReviewCompany {
	name: string;
	source: Source;
	board: string;
	reason: string | null;
	verified: boolean;
}

/** One company validation dropped, with the rule it broke. */
export interface DroppedCompany {
	name: string;
	source: string;
	board: string;
	why: string;
}

/** What a review card shows for a `plan_goal.plan` item. Every string is plain text. */
export interface PlanReview {
	badge: { label: string; tone: BadgeTone };
	goalId: number;
	version: number;
	goal: string;
	summary: string;
	companies: PlanReviewCompany[];
	dropped: { companies: DroppedCompany[]; keywords: string[]; locations: string[] };
}

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json =>
	typeof value === 'object' && value !== null && !Array.isArray(value);
const isString = (value: unknown): value is string => typeof value === 'string';
const isSource = (value: unknown): value is Source =>
	value === 'greenhouse' || value === 'lever' || value === 'ashby';

function parseCompany(value: unknown): PlanReviewCompany | null {
	if (!isObject(value)) return null;
	const { name, source, board, reason, verified } = value;
	if (!isString(name) || !isSource(source) || !isString(board)) return null;
	if (reason !== null && reason !== undefined && !isString(reason)) return null;
	return { name, source, board, reason: reason ?? null, verified: verified === true };
}

function parseDropped(value: unknown): DroppedCompany | null {
	if (!isObject(value)) return null;
	const { name, source, board, why } = value;
	if (!isString(name) || !isString(source) || !isString(board) || !isString(why)) return null;
	return { name, source, board, why };
}

/** The strings of `value` when it is an array of strings, else null. */
function strings(value: unknown): string[] | null {
	if (value === undefined) return [];
	return Array.isArray(value) && value.every(isString) ? value : null;
}

/**
 * What a review card shows for a `plan_goal.plan` item, or null for any other kind or a payload
 * that doesn't have the expected shape (the card then shows the raw JSON only).
 */
export function describePlanReview(item: ReviewItem): PlanReview | null {
	if (item.kind !== PLAN_KIND || !isObject(item.payload)) return null;
	const { goal_id, version, goal, summary, companies, dropped } = item.payload;
	if (typeof goal_id !== 'number' || typeof version !== 'number') return null;
	if (!isString(goal) || !isString(summary) || !Array.isArray(companies)) return null;
	const parsed = companies.map(parseCompany).filter((company) => company !== null);
	if (parsed.length !== companies.length) return null;
	const raw = isObject(dropped) ? dropped : {};
	const droppedCompanies = Array.isArray(raw.companies)
		? raw.companies.map(parseDropped).filter((company) => company !== null)
		: [];
	const keywords = strings(raw.keywords);
	const locations = strings(raw.locations);
	if (!keywords || !locations) return null;
	if (Array.isArray(raw.companies) && droppedCompanies.length !== raw.companies.length) return null;
	const droppedAny = droppedCompanies.length + keywords.length + locations.length > 0;
	return {
		badge: droppedAny
			? { label: 'Plan with dropped items', tone: 'warning' }
			: { label: 'Unsure plan', tone: 'warning' },
		goalId: goal_id,
		version,
		goal,
		summary,
		companies: parsed,
		dropped: { companies: droppedCompanies, keywords, locations }
	};
}
