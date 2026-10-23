import type { FitBand, FitCitation, FitReason, ReviewItem } from '#lib/api';
import type { BadgeTone } from '#lib/components/ui';
import { TOPIC_LABELS } from '#lib/memories/topics.js';

/** A fit score that wasn't applied: unsure, or with citations that didn't check out. */
export const FIT_KIND = 'score_fit.score';

/** What a review card shows for a `score_fit.score` item. Every string is plain text. */
export interface FitReview {
	badge: { label: string; tone: BadgeTone };
	postingId: number;
	company: string;
	title: string;
	score: number;
	band: FitBand;
	summary: string;
	reasons: FitReason[];
}

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json =>
	typeof value === 'object' && value !== null && !Array.isArray(value);
const isString = (value: unknown): value is string => typeof value === 'string';
const isBand = (value: unknown): value is FitBand =>
	value === 'strong' || value === 'partial' || value === 'weak';
const isTopic = (value: unknown): value is keyof typeof TOPIC_LABELS =>
	isString(value) && Object.hasOwn(TOPIC_LABELS, value);

function parseCitation(value: unknown): FitCitation | null {
	if (!isObject(value)) return null;
	const { source, memory_id, topic, text } = value;
	if (!isString(text)) return null;
	if (source === 'posting') {
		return { source, memory_id: null, topic: null, text };
	}
	if (source === 'memory' && typeof memory_id === 'number' && (topic === null || isTopic(topic))) {
		return { source, memory_id, topic, text };
	}
	return null;
}

function parseReason(value: unknown): FitReason | null {
	if (!isObject(value)) return null;
	const { stance, text, citations } = value;
	if ((stance !== 'for' && stance !== 'against') || !isString(text)) return null;
	if (!Array.isArray(citations)) return null;
	const parsed = citations.map(parseCitation).filter((citation) => citation !== null);
	if (parsed.length !== citations.length) return null;
	return { stance, text, citations: parsed };
}

/** Whether the payload says validation dropped any reason or citation. */
function droppedAny(value: unknown): boolean {
	if (!isObject(value)) return false;
	const { reasons, citations } = value;
	return (
		(typeof reasons === 'number' && reasons > 0) || (typeof citations === 'number' && citations > 0)
	);
}

/**
 * What a review card shows for a `score_fit.score` item, or null for any other kind or a
 * payload that doesn't have the expected shape (the card then shows the raw JSON only).
 */
export function describeFitReview(item: ReviewItem): FitReview | null {
	if (item.kind !== FIT_KIND || !isObject(item.payload)) return null;
	const { posting_id, company, title, score, band, summary, reasons, dropped } = item.payload;
	if (typeof posting_id !== 'number' || !isString(company) || !isString(title)) return null;
	if (typeof score !== 'number' || !isBand(band) || !isString(summary)) return null;
	if (!Array.isArray(reasons)) return null;
	const parsed = reasons.map(parseReason).filter((reason) => reason !== null);
	if (parsed.length !== reasons.length) return null;
	return {
		badge: droppedAny(dropped)
			? { label: 'Unchecked citations', tone: 'warning' }
			: { label: 'Unsure fit score', tone: 'warning' },
		postingId: posting_id,
		company,
		title,
		score,
		band,
		summary,
		reasons: parsed
	};
}
