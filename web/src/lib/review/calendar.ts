import type { PendingAction } from '#lib/api';

/** The `external` tool that adds an approved interview to Google Calendar. */
export const CALENDAR_TOOL = 'create_calendar_event';

/** What a pending calendar action shows. Every string is plain text from the tool input. */
export interface CalendarProposal {
	/** The event's title. */
	summary: string;
	/** The date-time range, formatted in the event's zone (or the browser's). */
	when: string;
	/** The event's IANA zone or `UTC`, or "your time zone" when it has none. */
	zone: string;
	/** The event's description, one entry per non-blank line. */
	description: string[];
}

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json =>
	typeof value === 'object' && value !== null && !Array.isArray(value);

/** An RFC 3339 date-time, as the tool takes; `Date` alone parses far looser text. */
const RFC_3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/i;

function date(value: unknown): Date | null {
	if (typeof value !== 'string' || !RFC_3339.test(value)) return null;
	const parsed = new Date(value);
	return Number.isNaN(parsed.getTime()) ? null : parsed;
}

/**
 * What the review card shows for a `create_calendar_event` action: the approved input when the
 * user edited it, else the requested one. Null for any other tool or an input that doesn't
 * have the expected shape (the card then shows the raw JSON only).
 */
export function describeCalendarAction(
	action: PendingAction,
	locale?: string
): CalendarProposal | null {
	if (action.tool !== CALENDAR_TOOL) return null;
	const input = action.approved_input ?? action.input;
	if (!isObject(input)) return null;
	const { summary, description, start, end, time_zone } = input;
	if (typeof summary !== 'string' || typeof description !== 'string') return null;
	if (time_zone !== null && typeof time_zone !== 'string') return null;
	const from = date(start);
	const to = date(end);
	if (!from || !to) return null;
	let when: string;
	try {
		when = new Intl.DateTimeFormat(locale, {
			dateStyle: 'medium',
			timeStyle: 'short',
			timeZone: time_zone ?? undefined
		}).formatRange(from, to);
	} catch {
		// An unknown zone or a range the browser can't format.
		return null;
	}
	return {
		summary,
		when,
		zone: time_zone ?? 'your time zone',
		description: description.split('\n').filter((line) => line.trim() !== '')
	};
}
