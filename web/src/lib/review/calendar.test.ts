import { describe, expect, it } from 'vitest';
import type { PendingAction } from '#lib/api';
import { CALENDAR_TOOL, describeCalendarAction } from './calendar';

const input = {
	summary: 'Interview: Hooli — Technical interview',
	description:
		'Role: Backend Engineer\nStage: Technical interview\nAdded by Waypoint after your approval.',
	start: '2030-11-03T19:00:00Z',
	end: '2030-11-03T19:45:00Z',
	time_zone: 'America/New_York'
};

function action(fields: Partial<PendingAction> = {}): PendingAction {
	return {
		id: 1,
		run_id: 2,
		run_step_id: 3,
		tool_call_id: 4,
		tool: CALENDAR_TOOL,
		input,
		approved_input: null,
		status: 'pending',
		output: null,
		executed_tool_call_id: null,
		created_at: '2030-11-01T00:00:00Z',
		decided_at: null,
		executed_at: null,
		...fields
	};
}

const range = (start: string, end: string, timeZone?: string) =>
	new Intl.DateTimeFormat('en-US', {
		dateStyle: 'medium',
		timeStyle: 'short',
		timeZone
	}).formatRange(new Date(start), new Date(end));

describe('describeCalendarAction', () => {
	it('reads a valid input in its own zone', () => {
		const proposal = describeCalendarAction(action(), 'en-US');
		expect(proposal).toEqual({
			summary: input.summary,
			when: range(input.start, input.end, 'America/New_York'),
			zone: 'America/New_York',
			description: [
				'Role: Backend Engineer',
				'Stage: Technical interview',
				'Added by Waypoint after your approval.'
			]
		});
		// 19:00 UTC is 2 PM in New York.
		expect(proposal?.when).toMatch(/2:00/);
	});

	it('uses the browser zone and says so without a time_zone', () => {
		const proposal = describeCalendarAction(
			action({ input: { ...input, time_zone: null } }),
			'en-US'
		);
		expect(proposal?.zone).toBe('your time zone');
		expect(proposal?.when).toBe(range(input.start, input.end));
	});

	it('prefers the approved input', () => {
		const approved_input = { ...input, summary: 'Edited', description: 'One\n\n  \nTwo' };
		const proposal = describeCalendarAction(action({ approved_input }), 'en-US');
		expect(proposal?.summary).toBe('Edited');
		expect(proposal?.description).toEqual(['One', 'Two']);
	});

	const invalid: [string, Partial<PendingAction>][] = [
		['another tool', { tool: 'draft_follow_up' }],
		['an input that is not an object', { input: 'nope' }],
		['an array input', { input: [input] }],
		['a missing summary', { input: { ...input, summary: undefined } }],
		['a numeric summary', { input: { ...input, summary: 42 } }],
		['a missing description', { input: { ...input, description: undefined } }],
		['a numeric time zone', { input: { ...input, time_zone: 5 } }],
		['a missing time zone', { input: { ...input, time_zone: undefined } }],
		['an unparseable start', { input: { ...input, start: 'Tuesday at 2' } }],
		['an impossible date', { input: { ...input, end: '2030-13-45T19:00:00Z' } }],
		['a missing end', { input: { ...input, end: undefined } }],
		['an unknown zone', { input: { ...input, time_zone: 'Mars/Olympus_Mons' } }]
	];

	it.each(invalid)('returns null for %s', (_, fields) => {
		expect(describeCalendarAction(action(fields), 'en-US')).toBeNull();
	});
});
