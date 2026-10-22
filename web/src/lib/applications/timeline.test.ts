import { describe, expect, it } from 'vitest';
import type { ApplicationEvent } from '#lib/api';
import { describeEvent } from './timeline';

const base: ApplicationEvent = {
	id: 1,
	application_id: 3,
	kind: 'note',
	from_status: null,
	to_status: null,
	stage_name: null,
	body: null,
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
	[{ kind: 'rejection_feedback', body: 'Too junior' }, 'Rejection feedback']
];

describe('describeEvent', () => {
	it.each(cases)('%o reads "%s"', (fields, want) => {
		expect(describeEvent({ ...base, ...fields })).toBe(want);
	});
});
