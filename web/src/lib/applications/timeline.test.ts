import { describe, expect, it } from 'vitest';
import type { ApplicationEvent, SourceEmail } from '#lib/api';
import { formatDate } from './format';
import { describeEvent, describeSource } from './timeline';

const base: ApplicationEvent = {
	id: 1,
	application_id: 3,
	kind: 'note',
	from_status: null,
	to_status: null,
	stage_name: null,
	body: null,
	source_email: null,
	created_at: '2026-10-22T15:04:00Z'
};

const cases: [Partial<ApplicationEvent>, string][] = [
	[{ kind: 'created', to_status: 'applied' }, 'Added as Applied'],
	[{ kind: 'created', to_status: null }, 'Added'],
	[
		{
			kind: 'status_changed',
			from_status: 'interviewing',
			to_status: 'rejected'
		},
		'Status changed from Interviewing to Rejected'
	],
	[{ kind: 'stage_added', stage_name: 'Onsite' }, 'Stage added: Onsite'],
	[{ kind: 'stage_completed', stage_name: 'Onsite' }, 'Stage completed: Onsite'],
	[{ kind: 'stage_reopened', stage_name: 'Onsite' }, 'Stage reopened: Onsite'],
	[{ kind: 'stage_removed', stage_name: 'Recruiter screen' }, 'Stage removed: Recruiter screen'],
	[{ kind: 'note', body: 'Called back' }, 'Note'],
	[{ kind: 'rejection_feedback', body: 'Too junior' }, 'Rejection feedback'],
	[
		{
			kind: 'interview_scheduled',
			body: 'Interview: Hooli — Onsite · 2030-11-03 14:00–14:45 America/New_York'
		},
		'Interview added to Google Calendar'
	]
];

describe('describeEvent', () => {
	it.each(cases)('%o reads "%s"', (fields, want) => {
		expect(describeEvent({ ...base, ...fields })).toBe(want);
	});
});

const email: SourceEmail = {
	id: 7,
	subject: 'Update on your application',
	from_name: 'Vandelay Industries',
	from_address: 'no-reply@ashbyhq.com',
	sent_at: '2026-09-10T21:45:00Z'
};
const sent = formatDate(email.sent_at!);

const sourceCases: [string, SourceEmail | null, string | null][] = [
	['no source email', null, null],
	[
		'subject, display name and date',
		email,
		`From email: "Update on your application" · Vandelay Industries · ${sent}`
	],
	[
		'the address without a display name',
		{ ...email, from_name: null },
		`From email: "Update on your application" · no-reply@ashbyhq.com · ${sent}`
	],
	[
		'a blank subject',
		{ ...email, subject: '  ' },
		`From email: "(no subject)" · Vandelay Industries · ${sent}`
	],
	[
		'no date',
		{ ...email, sent_at: null },
		'From email: "Update on your application" · Vandelay Industries'
	],
	[
		'no sender and no date',
		{ ...email, from_name: null, from_address: null, sent_at: null },
		'From email: "Update on your application"'
	]
];

describe('describeSource', () => {
	it.each(sourceCases)('%s', (_, source_email, want) => {
		expect(describeSource({ ...base, kind: 'status_changed', source_email })).toBe(want);
	});
});
