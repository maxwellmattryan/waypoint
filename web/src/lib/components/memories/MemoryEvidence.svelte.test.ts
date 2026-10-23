import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { MemorySignal } from '#lib/api';
import type { Result } from '#lib/memories/api.js';
import MemoryEvidence from './MemoryEvidence.svelte';

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

/**
 * Renders the panel with a loader whose requests stay pending until `settle()` resolves the
 * latest one with the next of `results`, so a test can look at the loading state first.
 */
function setup(...results: Result<MemorySignal[]>[]) {
	const pending: ((result: Result<MemorySignal[]>) => void)[] = [];
	const load = vi.fn<(id: number) => Promise<Result<MemorySignal[]>>>(
		() => new Promise((resolve) => pending.push(resolve))
	);
	render(MemoryEvidence, { memoryId: 9, load });
	const settle = () => {
		const next = results.shift();
		if (!next) throw new Error('no result queued');
		pending[pending.length - 1](next);
	};
	const toggle = () => screen.getByRole('button', { name: /evidence$/ });
	return { load, settle, toggle };
}

describe('MemoryEvidence', () => {
	it('loads once on first expand and lists each signal', async () => {
		const signals = [
			signal({
				id: 8,
				kind: 'dismissal',
				run_id: 21,
				pending_action_id: null,
				review_item_id: 4,
				subject: 'role_match',
				approved: null,
				changes: null,
				reason: 'Wrong city'
			}),
			signal({ id: 5 })
		];
		const { load, settle, toggle } = setup({ kind: 'ok', data: signals });
		expect(toggle()).toHaveAttribute('aria-expanded', 'false');
		expect(toggle()).toHaveTextContent('Show evidence');
		expect(load).not.toHaveBeenCalled();

		await userEvent.click(toggle());
		expect(load).toHaveBeenCalledExactlyOnceWith(9);
		expect(toggle()).toHaveAttribute('aria-expanded', 'true');
		expect(toggle()).toHaveTextContent('Hide evidence');
		const panel = document.getElementById(toggle().getAttribute('aria-controls') ?? '');
		const busy = screen.getByLabelText('Loading evidence');
		expect(busy).toHaveAttribute('aria-busy', 'true');
		expect(panel).toContainElement(busy);

		settle();
		const list = await screen.findByRole('list', { name: 'Evidence' });
		const rows = within(list).getAllByRole('listitem');
		expect(rows).toHaveLength(2);
		expect(within(rows[0]).getByText('Dismissed')).toBeInTheDocument();
		expect(within(rows[0]).getByText('role_match')).toBeInTheDocument();
		expect(within(rows[0]).getByText('Wrong city')).toBeInTheDocument();
		expect(within(rows[0]).getByRole('link', { name: 'Run #21' })).toBeInTheDocument();
		expect(within(rows[1]).getByText('Approved')).toBeInTheDocument();
		expect(within(rows[1]).getByText('draft_follow_up')).toBeInTheDocument();
		expect(within(rows[1]).getByRole('link', { name: 'Run #12' })).toBeInTheDocument();

		await userEvent.click(toggle());
		expect(toggle()).toHaveAttribute('aria-expanded', 'false');
		await userEvent.click(toggle());
		expect(toggle()).toHaveAttribute('aria-expanded', 'true');
		expect(await screen.findByRole('list', { name: 'Evidence' })).toBeInTheDocument();
		expect(load).toHaveBeenCalledOnce();
	});

	it('shows the empty state', async () => {
		const { settle, toggle } = setup({ kind: 'ok', data: [] });
		await userEvent.click(toggle());
		settle();
		expect(
			await screen.findByText('No decisions are linked to this memory any more.')
		).toBeInTheDocument();
		expect(screen.queryByRole('list', { name: 'Evidence' })).toBeNull();
	});

	it('shows the error state and retries', async () => {
		const { load, settle, toggle } = setup(
			{ kind: 'failed' },
			{ kind: 'ok', data: [signal({ subject: 'send_email' })] }
		);
		await userEvent.click(toggle());
		settle();
		expect(await screen.findByText("Couldn't load the evidence.")).toBeInTheDocument();

		await userEvent.click(screen.getByRole('button', { name: 'Try again' }));
		expect(load).toHaveBeenCalledTimes(2);
		expect(load).toHaveBeenLastCalledWith(9);
		expect(screen.getByLabelText('Loading evidence')).toHaveAttribute('aria-busy', 'true');
		expect(screen.queryByText("Couldn't load the evidence.")).toBeNull();

		settle();
		const list = await screen.findByRole('list', { name: 'Evidence' });
		expect(within(list).getByText('send_email')).toBeInTheDocument();
	});
});
