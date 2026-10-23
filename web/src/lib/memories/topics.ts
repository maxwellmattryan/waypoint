import { resolve } from '$app/paths';
import type { Memory, MemoryKind, MemoryStatus, MemoryTopic } from '#lib/api';

/** Each kind's name in the memory dialog's Kind select. */
export const KIND_LABELS = {
	fact: 'Fact',
	preference: 'Preference',
	answer: 'Answer'
} as const satisfies Record<MemoryKind, string>;

/** The page's sections, in order, with their headings. */
export const SECTIONS: readonly { kind: MemoryKind; heading: string }[] = [
	{ kind: 'preference', heading: 'Preferences' },
	{ kind: 'answer', heading: 'Answer bank' },
	{ kind: 'fact', heading: 'Facts' }
];

/** Every topic's label. `satisfies` makes a topic added in the API break the type check here. */
export const TOPIC_LABELS = {
	position: 'Position',
	skill: 'Skill',
	education: 'Education',
	target_role: 'Target role',
	seniority: 'Seniority',
	location: 'Location',
	comp_floor: 'Comp floor',
	industry: 'Industry',
	deal_breaker: 'Deal breaker',
	work_authorization: 'Work authorization',
	notice_period: 'Notice period',
	salary_expectation: 'Salary expectation',
	why_leaving: 'Why leaving',
	other: 'Other'
} as const satisfies Record<MemoryTopic, string>;

/** The topics each kind allows, in display order; mirrors `MemoryKind::topics` in the API. */
export const TOPICS_BY_KIND = {
	fact: ['position', 'skill', 'education', 'other'],
	preference: [
		'target_role',
		'seniority',
		'location',
		'comp_floor',
		'industry',
		'deal_breaker',
		'other'
	],
	answer: ['work_authorization', 'notice_period', 'salary_expectation', 'why_leaving', 'other']
} as const satisfies Record<MemoryKind, readonly MemoryTopic[]>;

/** The status filter's options; `''` is "All". */
export const STATUS_OPTIONS: { value: MemoryStatus | ''; label: string }[] = [
	{ value: '', label: 'All' },
	{ value: 'proposed', label: 'Proposed' },
	{ value: 'confirmed', label: 'Confirmed' }
];

/** Whether `topic` is one of `kind`'s topics. */
export function topicAllowed(kind: MemoryKind, topic: MemoryTopic): boolean {
	return (TOPICS_BY_KIND[kind] as readonly MemoryTopic[]).includes(topic);
}

/** Where a memory came from, as one line; `href` only for a numeric reference. */
export interface SourceLine {
	text: string;
	href?: string;
}

const DIGITS = /^\d+$/;

/**
 * The provenance line: "Added by you", or "From <source>" plus the label. A resume or run
 * without a label shows `#<ref>` instead, and links to its page, but only when the reference
 * is all digits, so no stored string ever reaches an `href`.
 */
export function sourceLine(
	memory: Pick<Memory, 'source_kind' | 'source_ref' | 'source_label'>
): SourceLine {
	const ref = memory.source_ref?.trim() || null;
	const label = memory.source_label?.trim() || null;
	const line = (prefix: string, fallback: string | null = null) => {
		const detail = label ?? fallback;
		return detail ? `${prefix} ${detail}` : prefix;
	};
	const id = ref && DIGITS.test(ref) ? ref : null;
	const numbered = ref ? `#${ref}` : null;
	switch (memory.source_kind) {
		case 'manual':
			return { text: 'Added by you' };
		case 'resume': {
			const text = line('From resume', numbered);
			return id ? { text, href: resolve('/resumes/[id]', { id }) } : { text };
		}
		case 'run': {
			const text = line('From run', numbered);
			return id ? { text, href: resolve('/runs/[id]', { id }) } : { text };
		}
		case 'linkedin':
			return { text: line('From LinkedIn export') };
		case 'email':
			return { text: line('From email') };
	}
}
