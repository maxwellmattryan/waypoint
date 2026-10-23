import { describe, expect, it } from 'vitest';
import type { ReviewItem } from '#lib/api';
import { formatDate } from '#lib/applications/format.js';
import { describeEmailReview, type EmailReview } from './email';

const email = {
	subject: 'Invitation: Interview with Stark Industries',
	from_name: 'Stark Scheduling',
	from_address: 'calendar@stark.example',
	sent_at: '2026-09-18T12:00:00Z'
};
const sent = formatDate(email.sent_at);

const candidate = (id: number, title: string, score: number) => ({
	application_id: id,
	company: 'Stark Industries',
	title,
	status: 'applied',
	score,
	reasons: ['company']
});

const match = {
	email_id: 3,
	classification_id: 7,
	class: 'interview_invite',
	email,
	application_id: 12,
	company: 'Stark Industries',
	title: 'Software Engineer',
	ops: [
		{ op: 'set_status', from: 'applied', to: 'interviewing' },
		{ op: 'add_stage', name: 'Technical interview' }
	],
	reasons: ['company', 'role'],
	review_reasons: ['low_confidence'],
	candidates: [candidate(12, 'Software Engineer', 0.8)]
};

const proposal = {
	email_id: 4,
	classification_id: 8,
	class: 'other',
	email: { ...email, subject: 'Thank you for applying!', from_name: 'Globex' },
	application: { company: 'Globex', title: 'Data Engineer', status: 'applied' }
};

function item(kind: string, payload: unknown): ReviewItem {
	return {
		id: 1,
		run_id: 2,
		run_step_id: 3,
		kind,
		confidence: 0.6,
		payload,
		status: 'pending',
		created_at: '2026-10-22T00:00:00Z',
		resolved_at: null
	};
}

const summary: EmailReview = {
	kind: 'match',
	badge: { label: 'Low confidence', tone: 'warning' },
	email: { subject: email.subject, sender: 'Stark Scheduling', date: sent },
	target: 'Stark Industries — Software Engineer',
	company: 'Stark Industries',
	changes: ['Status: Applied → Interviewing', 'Add stage: Technical interview'],
	reasons: ['Company', 'Role'],
	candidates: []
};

const cases: [string, ReviewItem, EmailReview | null][] = [
	['a low-confidence match', item('email.match', match), summary],
	[
		'an ambiguous match lists the other candidates',
		item('email.match', {
			...match,
			reasons: ['company', 'sender_domain'],
			review_reasons: ['low_confidence', 'ambiguous'],
			candidates: [candidate(12, 'Software Engineer', 0.5), candidate(13, 'Data Engineer', 0.5)]
		}),
		{
			...summary,
			badge: { label: 'Ambiguous match', tone: 'warning' },
			reasons: ['Company', 'Sender domain'],
			candidates: ['Stark Industries — Data Engineer (50%)']
		}
	],
	[
		'an unexpected status outranks the other reasons',
		item('email.match', {
			...match,
			ops: [{ op: 'set_status', from: 'rejected', to: 'offer' }],
			reasons: ['thread', 'role_partial', 'mystery'],
			review_reasons: ['low_confidence', 'unexpected_status']
		}),
		{
			...summary,
			badge: { label: 'Unexpected status', tone: 'danger' },
			changes: ['Status: Rejected → Offer'],
			reasons: ['Same thread', 'Role (partial)', 'mystery']
		}
	],
	[
		'a blank subject, no display name and no date',
		item('email.match', {
			...match,
			email: { ...email, subject: ' ', from_name: null, sent_at: null }
		}),
		{
			...summary,
			email: {
				subject: '(no subject)',
				sender: 'calendar@stark.example',
				date: null
			}
		}
	],
	[
		'a proposed application',
		item('email.new_application', proposal),
		{
			kind: 'new_application',
			badge: { label: 'New application', tone: 'info' },
			email: {
				subject: 'Thank you for applying!',
				sender: 'Globex',
				date: sent
			},
			target: 'Globex — Data Engineer',
			company: 'Globex',
			changes: ['Create application as Applied'],
			reasons: [],
			candidates: []
		}
	],
	['another kind', item('tidy.note', { application_id: 1, note: 'n' }), null],
	['a payload that is no object', item('email.match', [match]), null],
	['no email', item('email.match', { ...match, email: null }), null],
	['an unknown op', item('email.match', { ...match, ops: [{ op: 'delete_application' }] }), null],
	[
		'an unknown status',
		item('email.match', {
			...match,
			ops: [{ op: 'set_status', from: 'applied', to: 'hired' }]
		}),
		null
	],
	[
		'a malformed candidate of an ambiguous match',
		item('email.match', {
			...match,
			review_reasons: ['ambiguous'],
			candidates: [{ score: 1 }]
		}),
		null
	],
	[
		'a proposal without a status',
		item('email.new_application', {
			...proposal,
			application: { company: 'Globex', title: 'Data Engineer' }
		}),
		null
	]
];

describe('describeEmailReview', () => {
	it.each(cases)('%s', (_, reviewItem, want) => {
		expect(describeEmailReview(reviewItem)).toEqual(want);
	});
});
