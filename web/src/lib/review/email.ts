import type { ReviewItem } from '#lib/api';
import { formatDate } from '#lib/applications/format.js';
import { isStatus, statusLabel } from '#lib/applications/statuses.js';
import { emailSender, emailSubject } from '#lib/applications/timeline.js';
import type { BadgeTone } from '#lib/components/ui';

/** An unsure match of an email to an existing application. */
export const MATCH_KIND = 'email.match';
/** An email that proposes a new application. */
export const NEW_APPLICATION_KIND = 'email.new_application';

/** What a review card shows for an email review item. Every string is plain text. */
export interface EmailReview {
	kind: 'match' | 'new_application';
	badge: { label: string; tone: BadgeTone };
	email: {
		subject: string;
		/** The display name, else the address; null when the email has neither. */
		sender: string | null;
		/** The sent date; null when unknown. */
		date: string | null;
	};
	/** "Company — Title": the matched application, or the proposed one. */
	target: string;
	/** The application's company, for the Apply toast. */
	company: string;
	/** One line per proposed change, e.g. "Status: Applied → Rejected". */
	changes: string[];
	/** The match reasons, readable. */
	reasons: string[];
	/** For an ambiguous match, the other candidates: "Company — Title (80%)". */
	candidates: string[];
}

const REASONS: Record<string, string> = {
	thread: 'Same thread',
	company: 'Company',
	sender_name: 'Sender name',
	sender_domain: 'Sender domain',
	role: 'Role',
	role_partial: 'Role (partial)',
	role_mismatch: 'Role mismatch'
};

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json =>
	typeof value === 'object' && value !== null && !Array.isArray(value);
const isString = (value: unknown): value is string => typeof value === 'string';
const optionalString = (value: unknown): value is string | null =>
	value === null || isString(value);

function parseEmail(value: unknown): EmailReview['email'] | null {
	if (!isObject(value)) return null;
	const { subject, from_name, from_address, sent_at } = value;
	if (!isString(subject) || !optionalString(from_name) || !optionalString(from_address)) {
		return null;
	}
	if (!optionalString(sent_at)) return null;
	const date = sent_at && !Number.isNaN(Date.parse(sent_at)) ? formatDate(sent_at) : null;
	return {
		subject: emailSubject(subject),
		sender: emailSender(from_name, from_address),
		date
	};
}

const target = (company: string, title: string) => `${company} — ${title}`;

function parseChange(op: unknown): string | null {
	if (!isObject(op)) return null;
	if (op.op === 'set_status' && isString(op.from) && isString(op.to)) {
		if (!isStatus(op.from) || !isStatus(op.to)) return null;
		return `Status: ${statusLabel[op.from]} → ${statusLabel[op.to]}`;
	}
	if (op.op === 'add_stage' && isString(op.name)) return `Add stage: ${op.name}`;
	return null;
}

function parseCandidate(value: unknown): { id: number; line: string } | null {
	if (!isObject(value)) return null;
	const { application_id, company, title, score } = value;
	if (typeof application_id !== 'number' || !isString(company) || !isString(title)) return null;
	if (typeof score !== 'number') return null;
	return {
		id: application_id,
		line: `${target(company, title)} (${Math.round(score * 100)}%)`
	};
}

/** The badge of an `email.match` item: its most serious review reason. */
function matchBadge(reviewReasons: string[]): EmailReview['badge'] {
	if (reviewReasons.includes('unexpected_status')) {
		return { label: 'Unexpected status', tone: 'danger' };
	}
	if (reviewReasons.includes('ambiguous')) return { label: 'Ambiguous match', tone: 'warning' };
	return { label: 'Low confidence', tone: 'warning' };
}

function describeMatch(payload: Json, email: EmailReview['email']): EmailReview | null {
	const { application_id, company, title, ops, reasons, review_reasons, candidates } = payload;
	if (typeof application_id !== 'number' || !isString(company) || !isString(title)) return null;
	if (!Array.isArray(ops) || !Array.isArray(reasons) || !Array.isArray(review_reasons)) {
		return null;
	}
	if (!reasons.every(isString) || !review_reasons.every(isString)) return null;
	const changes = ops.map(parseChange).filter((change) => change !== null);
	if (changes.length !== ops.length) return null;
	let others: string[] = [];
	if (review_reasons.includes('ambiguous')) {
		if (!Array.isArray(candidates)) return null;
		const parsed = candidates.map(parseCandidate).filter((candidate) => candidate !== null);
		if (parsed.length !== candidates.length) return null;
		others = parsed
			.filter((candidate) => candidate.id !== application_id)
			.map((candidate) => candidate.line);
	}
	return {
		kind: 'match',
		badge: matchBadge(review_reasons),
		email,
		target: target(company, title),
		company,
		changes,
		reasons: reasons.map((reason) => REASONS[reason] ?? reason),
		candidates: others
	};
}

function describeNew(payload: Json, email: EmailReview['email']): EmailReview | null {
	const { application } = payload;
	if (!isObject(application)) return null;
	const { company, title, status } = application;
	if (!isString(company) || !isString(title) || !isString(status) || !isStatus(status)) {
		return null;
	}
	return {
		kind: 'new_application',
		badge: { label: 'New application', tone: 'info' },
		email,
		target: target(company, title),
		company,
		changes: [`Create application as ${statusLabel[status]}`],
		reasons: [],
		candidates: []
	};
}

/**
 * What a review card shows for an `email.match` or `email.new_application` item, or null for
 * any other kind or a payload that doesn't have the expected shape (the card then shows the
 * raw JSON only).
 */
export function describeEmailReview(item: ReviewItem): EmailReview | null {
	if (item.kind !== MATCH_KIND && item.kind !== NEW_APPLICATION_KIND) return null;
	if (!isObject(item.payload)) return null;
	const email = parseEmail(item.payload.email);
	if (!email) return null;
	return item.kind === MATCH_KIND
		? describeMatch(item.payload, email)
		: describeNew(item.payload, email);
}
