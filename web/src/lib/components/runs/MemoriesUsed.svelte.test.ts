import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { RunMemory } from '#lib/api';
import MemoriesUsed from './MemoriesUsed.svelte';

function memory(overrides: Partial<RunMemory> = {}): RunMemory {
	return {
		id: 1,
		run_id: 9,
		run_step_id: 30,
		step: 'draft',
		memory_id: 4,
		kind: 'preference',
		topic: 'location',
		content: 'Remote only',
		score: 0.6,
		created_at: '2026-10-22T10:00:00Z',
		...overrides
	};
}

describe('MemoriesUsed', () => {
	it('lists each memory with its topic, snapshot content and step, in order', () => {
		render(MemoriesUsed, {
			memories: [
				memory(),
				memory({
					id: 2,
					memory_id: null,
					kind: 'answer',
					topic: 'notice_period',
					content: 'Two weeks\nnegotiable',
					step: 'note'
				})
			]
		});
		const items = within(screen.getByRole('list', { name: 'Memories used' })).getAllByRole(
			'listitem'
		);
		expect(items).toHaveLength(2);

		const [live, deleted] = items;
		expect(within(live).getByText('Location')).toBeInTheDocument();
		expect(within(live).getByText('Remote only')).toBeInTheDocument();
		expect(within(live).getByText('Used in draft')).toBeInTheDocument();
		expect(within(live).getByRole('link', { name: 'View memory' })).toHaveAttribute(
			'href',
			'/memories?memory=4'
		);
		expect(within(live).queryByText('Deleted')).toBeNull();

		expect(within(deleted).getByText('Notice period')).toBeInTheDocument();
		expect(within(deleted).getByText(/Two weeks/).textContent).toBe('Two weeks\nnegotiable');
		expect(within(deleted).getByText('Used in note')).toBeInTheDocument();
		expect(within(deleted).getByText('Deleted')).toBeInTheDocument();
		expect(within(deleted).queryByRole('link')).toBeNull();
	});

	it('renders content as text', () => {
		const { container } = render(MemoriesUsed, {
			memories: [memory({ content: '<img src=x onerror=alert(1)>' })]
		});
		expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument();
		expect(container.querySelector('img')).toBeNull();
	});

	it('shows the empty state when the run used no memories', () => {
		render(MemoriesUsed, { memories: [] });
		expect(screen.getByText('No memories used')).toBeInTheDocument();
		expect(screen.getByText("This run didn't use any memories.")).toBeInTheDocument();
		expect(screen.queryByRole('list')).toBeNull();
	});
});
