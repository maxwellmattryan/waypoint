import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Memory } from '#lib/api';
import MemoryItem from './MemoryItem.svelte';

function memory(overrides: Partial<Memory> = {}): Memory {
	return {
		id: 4,
		kind: 'preference',
		topic: 'comp_floor',
		content: 'At least 180k base\nplus equity',
		tags: ['comp', 'us'],
		status: 'confirmed',
		confirmed_at: '2026-10-22T10:00:00Z',
		source_kind: 'manual',
		source_ref: null,
		source_label: null,
		created_at: '2026-10-22T10:00:00Z',
		updated_at: '2026-10-22T10:00:00Z',
		...overrides
	};
}

function setup(overrides: Partial<Memory> = {}) {
	const handlers = {
		onconfirm: vi.fn(async () => true),
		onedit: vi.fn(),
		ondelete: vi.fn(),
		ontag: vi.fn()
	};
	const { container } = render(MemoryItem, { memory: memory(overrides), ...handlers });
	return { ...handlers, container, source: () => screen.getByTestId('source') };
}

describe('MemoryItem', () => {
	it('shows the topic label, content with its line breaks, tags and dates', async () => {
		const { container, ontag } = setup();
		expect(screen.getByText('Comp floor')).toBeInTheDocument();
		const content = screen.getByText(/At least 180k base/);
		expect(content.textContent).toBe('At least 180k base\nplus equity');
		expect(content).toHaveClass('whitespace-pre-line');
		expect(screen.getByText(/^Added (?!by you)/)).toBeInTheDocument();
		expect(screen.queryByText(/^Updated /)).toBeNull();
		const tags = within(screen.getByRole('list', { name: 'Tags' })).getAllByRole('button');
		expect(tags.map((t) => t.textContent?.trim())).toEqual(['comp', 'us']);
		await userEvent.click(screen.getByRole('button', { name: 'us' }));
		expect(ontag).toHaveBeenCalledWith('us');
		expect(container.querySelector('a')).toBeNull();
	});

	it('shows "Updated" when updated_at differs from created_at', () => {
		setup({ updated_at: '2026-10-23T10:00:00Z' });
		expect(screen.getByText(/^Updated /)).toBeInTheDocument();
	});

	it('renders markup in content as text', () => {
		const { container } = setup({ content: '<img src=x onerror=alert(1)><b>bold</b>' });
		expect(screen.getByText('<img src=x onerror=alert(1)><b>bold</b>')).toBeInTheDocument();
		expect(container.querySelector('img')).toBeNull();
		expect(container.querySelector('b')).toBeNull();
	});

	it('shows the Proposed badge and Confirm only when proposed', async () => {
		const { onconfirm } = setup({ status: 'proposed', confirmed_at: null });
		expect(screen.getByText('Proposed')).toBeInTheDocument();
		await userEvent.click(screen.getByRole('button', { name: 'Confirm' }));
		expect(onconfirm).toHaveBeenCalledWith(expect.objectContaining({ id: 4 }));
	});

	it('has no badge or Confirm when confirmed, and Edit and Delete either way', async () => {
		const { onedit, ondelete } = setup();
		expect(screen.queryByText('Proposed')).toBeNull();
		expect(screen.queryByRole('button', { name: 'Confirm' })).toBeNull();
		await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
		await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
		expect(onedit).toHaveBeenCalledWith(expect.objectContaining({ id: 4 }));
		expect(ondelete).toHaveBeenCalledWith(expect.objectContaining({ id: 4 }));
	});

	it.each([
		[{ source_kind: 'manual' }, 'Added by you', null],
		[
			{ source_kind: 'resume', source_ref: '12', source_label: 'Resume · Staff Eng v3' },
			'From resume Resume · Staff Eng v3',
			'/resumes/12'
		],
		[{ source_kind: 'resume', source_ref: '12' }, 'From resume #12', '/resumes/12'],
		[{ source_kind: 'run', source_ref: '7' }, 'From run #7', '/runs/7'],
		[{ source_kind: 'run', source_ref: '7', source_label: 'Run #7 · tidy' }, 'From run Run #7 · tidy', '/runs/7'],
		[{ source_kind: 'run', source_ref: 'javascript:alert(1)' }, 'From run #javascript:alert(1)', null],
		[{ source_kind: 'resume', source_ref: '../12' }, 'From resume #../12', null],
		[
			{ source_kind: 'linkedin', source_ref: '3', source_label: 'Export Oct 2026' },
			'From LinkedIn export Export Oct 2026',
			null
		],
		[
			{ source_kind: 'email', source_ref: '18f2a', source_label: 'Acme recruiter' },
			'From email Acme recruiter',
			null
		],
		[
			{ source_kind: 'signal', source_ref: '3', source_label: '2 decisions' },
			'Learned from 2 decisions',
			null
		],
		[{ source_kind: 'signal', source_ref: '3' }, 'Learned from your decisions', null]
	] as const)('shows the provenance %o', (overrides, text, href) => {
		const { source } = setup(overrides as Partial<Memory>);
		expect(source().textContent?.trim()).toBe(text);
		const link = source().querySelector('a');
		if (href) expect(link).toHaveAttribute('href', href);
		else expect(link).toBeNull();
	});
});
