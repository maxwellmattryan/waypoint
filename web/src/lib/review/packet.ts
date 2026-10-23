import type { FitCitation, PacketQuestion, ReviewItem } from '#lib/api';
import type { BadgeTone } from '#lib/components/ui';
import { isObject, isString, parseCitation } from './fit';

/** An application packet that isn't ready: unsure, with missing answers or dropped citations. */
export const PACKET_KIND = 'prep_packet.packet';

/** One drafted answer on a packet review card. */
export interface PacketReviewAnswer {
	key: PacketQuestion;
	question: string;
	answer: string;
	citations: FitCitation[];
}

/** What a review card shows for a `prep_packet.packet` item. Every string is plain text. */
export interface PacketReview {
	badge: { label: string; tone: BadgeTone };
	postingId: number;
	company: string;
	title: string;
	resumeName: string;
	/** The questions left without an answer, as their text. */
	missing: string[];
	/** The drafted answers, in question order. */
	answers: PacketReviewAnswer[];
}

const KEYS: readonly PacketQuestion[] = [
	'work_authorization',
	'notice_period',
	'salary_expectation',
	'why_leaving',
	'why_company'
];

const isKey = (value: unknown): value is PacketQuestion =>
	isString(value) && (KEYS as readonly string[]).includes(value);

type Parsed = {
	key: PacketQuestion;
	question: string;
	answer: string | null;
	citations: FitCitation[];
};

function parseAnswer(value: unknown): Parsed | null {
	if (!isObject(value)) return null;
	const { key, question, answer, citations } = value;
	if (!isKey(key) || !isString(question) || (answer !== null && !isString(answer))) return null;
	if (!Array.isArray(citations)) return null;
	const parsed = citations.map(parseCitation).filter((citation) => citation !== null);
	if (parsed.length !== citations.length) return null;
	return { key, question, answer, citations: parsed };
}

/** Whether the payload says validation dropped anything. */
function droppedAny(value: unknown): boolean {
	if (!isObject(value)) return false;
	return Object.values(value).some((count) => typeof count === 'number' && count > 0);
}

/**
 * What a review card shows for a `prep_packet.packet` item, or null for any other kind or a
 * payload that doesn't have the expected shape (the card then shows the raw JSON only).
 */
export function describePacketReview(item: ReviewItem): PacketReview | null {
	if (item.kind !== PACKET_KIND || !isObject(item.payload)) return null;
	const { posting_id, company, title, resume, answers, dropped } = item.payload;
	if (typeof posting_id !== 'number' || !isString(company) || !isString(title)) return null;
	if (!isObject(resume) || !isString(resume.name)) return null;
	if (!Array.isArray(answers)) return null;
	const parsed = answers.map(parseAnswer).filter((answer) => answer !== null);
	if (parsed.length !== answers.length) return null;
	const missing = parsed.filter((answer) => answer.answer === null);
	const drafted: PacketReviewAnswer[] = parsed.flatMap((answer) =>
		answer.answer === null ? [] : [{ ...answer, answer: answer.answer }]
	);
	const badge: PacketReview['badge'] = droppedAny(dropped)
		? { label: 'Unchecked citations', tone: 'warning' }
		: missing.length > 0
			? { label: 'Missing answers', tone: 'warning' }
			: { label: 'Unsure packet', tone: 'warning' };
	return {
		badge,
		postingId: posting_id,
		company,
		title,
		resumeName: resume.name,
		missing: missing.map((answer) => answer.question),
		answers: drafted
	};
}
