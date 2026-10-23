import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { MemorySignal } from '#lib/api';
import SignalItem from './SignalItem.svelte';

function signal(overrides: Partial<MemorySignal> = {}): MemorySignal {
	return {
		id: 5,
		kind: 'approval',
		run_id: 12,
		pending_action_id: 3,
		review_item_id: null,
		subject: 'draft_follow_up',
		drafted: { message: 'Thanks' },
		approved: { message: 'Thanks' },
		changes: [],
		reason: null,
		created_at: '2026-10-22T00:00:00Z',
		...overrides
	};
}

describe('SignalItem', () => {
	it.each([
		['an unedited approval', signal(), 'Approved'],
		['a rejection', signal({ kind: 'rejection', approved: null, changes: null }), 'Rejected'],
		[
			'a dismissal',
			signal({
				kind: 'dismissal',
				pending_action_id: null,
				review_item_id: 4,
				subject: 'tidy.note',
				approved: null,
				changes: null
			}),
			'Dismissed'
		]
	])('badges %s "%s" with its subject and run', (_, value, label) => {
		render(SignalItem, { signal: value });
		expect(screen.getByText(label)).toBeInTheDocument();
		expect(screen.getByText(value.subject)).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Run #12' })).toBeInTheDocument();
		expect(screen.queryByRole('list', { name: 'Changes' })).not.toBeInTheDocument();
	});

	it('shows "Approved with edits" with one row per change', () => {
		render(SignalItem, {
			signal: signal({
				approved: { message: 'edited', tone: 'warm' },
				changes: [
					{
						op: 'replace',
						path: '/message',
						before: 'Thanks',
						after: 'edited'
					},
					{ op: 'add', path: '/tone', before: null, after: { level: 'warm' } },
					{ op: 'remove', path: '/cc', before: ['a'], after: null }
				]
			})
		});
		expect(screen.getByText('Approved with edits')).toBeInTheDocument();
		expect(screen.queryByText('Approved')).not.toBeInTheDocument();
		const rows = within(screen.getByRole('list', { name: 'Changes' })).getAllByRole('listitem');
		expect(rows).toHaveLength(3);
		expect(rows[0]).toHaveTextContent('replace');
		expect(within(rows[0]).getByText('/message')).toBeInTheDocument();
		expect(within(rows[0]).getByText('"Thanks"')).toBeInTheDocument();
		expect(within(rows[0]).getByText('"edited"')).toBeInTheDocument();
		expect(within(rows[1]).getByText('{"level":"warm"}')).toHaveAttribute(
			'title',
			'{"level":"warm"}'
		);
		expect(within(rows[1]).getByLabelText('none')).toBeInTheDocument();
		expect(within(rows[2]).getByText('["a"]')).toBeInTheDocument();
		expect(within(rows[2]).getByLabelText('none')).toBeInTheDocument();
	});

	it("shows a dismissal's reason only when it has one", () => {
		const dismissal = signal({
			kind: 'dismissal',
			pending_action_id: null,
			review_item_id: 4,
			subject: 'tidy.note',
			approved: null,
			changes: null,
			reason: 'Wrong company'
		});
		const { unmount } = render(SignalItem, { signal: dismissal });
		expect(screen.getByText('Wrong company')).toBeInTheDocument();
		unmount();

		render(SignalItem, { signal: { ...dismissal, reason: null } });
		expect(screen.getByText('Dismissed')).toBeInTheDocument();
		expect(screen.queryByText(/Reason:/)).not.toBeInTheDocument();
	});
});
