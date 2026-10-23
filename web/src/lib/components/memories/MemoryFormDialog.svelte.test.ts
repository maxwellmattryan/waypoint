import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Memory, MemoryInput } from '#lib/api';
import type { Result } from '#lib/memories/api.js';
import MemoryFormDialog from './MemoryFormDialog.svelte';

const stored: Memory = {
	id: 9,
	kind: 'answer',
	topic: 'notice_period',
	content: 'Four weeks',
	tags: ['notice'],
	status: 'proposed',
	confirmed_at: null,
	source_kind: 'resume',
	source_ref: '2',
	source_label: null,
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:00Z'
};

function setup(save: (input: MemoryInput) => Promise<Result<Memory>>, memory: Memory | null = null) {
	const user = userEvent.setup();
	const onsaved = vi.fn();
	render(MemoryFormDialog, { open: true, memory, save, onsaved });
	const options = (label: string) =>
		[...(screen.getByLabelText(label) as HTMLSelectElement).options].map((o) => o.value);
	return {
		user,
		onsaved,
		kind: () => screen.getByLabelText('Kind') as HTMLSelectElement,
		topic: () => screen.getByLabelText('Topic') as HTMLSelectElement,
		content: () => screen.getByLabelText('Content'),
		tags: () => screen.getByLabelText('Tags'),
		options,
		submit: () => user.click(screen.getByRole('button', { name: 'Save' }))
	};
}

describe('MemoryFormDialog', () => {
	it('adds: limits topics to the kind and resets the topic when the kind changes', async () => {
		const save = vi.fn(async () => ({ kind: 'ok', data: stored }) as Result<Memory>);
		const form = setup(save);
		expect(screen.getByRole('dialog', { name: 'Add memory' })).toBeInTheDocument();
		expect(form.kind().value).toBe('fact');
		expect(form.options('Topic')).toEqual(['position', 'skill', 'education', 'other']);

		await form.user.selectOptions(form.topic(), 'education');
		await form.user.selectOptions(form.kind(), 'preference');
		expect(form.topic().value).toBe('target_role');
		expect(form.options('Topic')).toEqual([
			'target_role',
			'seniority',
			'location',
			'comp_floor',
			'industry',
			'deal_breaker',
			'other'
		]);
		await form.user.selectOptions(form.topic(), 'comp_floor');
		await form.user.type(form.content(), 'At least 180k');
		await form.user.type(form.tags(), 'Comp{Enter}');
		await form.submit();

		expect(save).toHaveBeenCalledWith({
			kind: 'preference',
			topic: 'comp_floor',
			content: 'At least 180k',
			tags: ['comp']
		});
		expect(form.onsaved).toHaveBeenCalledWith(stored, true);
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
	});

	it('edits: prefilled from the memory, and onsaved says it was not new', async () => {
		const save = vi.fn(async () => ({ kind: 'ok', data: stored }) as Result<Memory>);
		const form = setup(save, stored);
		expect(screen.getByRole('dialog', { name: 'Edit memory' })).toBeInTheDocument();
		expect(form.kind().value).toBe('answer');
		expect(form.topic().value).toBe('notice_period');
		expect(form.content()).toHaveValue('Four weeks');
		expect(screen.getByRole('button', { name: 'Remove tag notice' })).toBeInTheDocument();
		await form.submit();
		expect(save).toHaveBeenCalledWith({
			kind: 'answer',
			topic: 'notice_period',
			content: 'Four weeks',
			tags: ['notice']
		});
		expect(form.onsaved).toHaveBeenCalledWith(stored, false);
	});

	it('shows each 422 error on its field and stays open', async () => {
		const save = vi.fn(
			async () =>
				({
					kind: 'invalid',
					errors: [
						{ field: 'content', message: 'Required' },
						{ field: 'topic', message: 'Not a topic for this kind' },
						{ field: 'tags', message: 'At most 20 tags' }
					]
				}) as Result<Memory>
		);
		const form = setup(save);
		await form.submit();
		expect(form.content()).toHaveAccessibleDescription('Required');
		expect(form.topic()).toHaveAccessibleDescription('Not a topic for this kind');
		expect(form.tags()).toHaveAccessibleDescription(/At most 20 tags/);
		expect(form.onsaved).not.toHaveBeenCalled();
		expect(screen.getByRole('dialog', { name: 'Add memory' })).toBeInTheDocument();
	});

	it('shows other failures inside the dialog and stays open', async () => {
		const save = vi.fn(async () => ({ kind: 'failed' }) as Result<Memory>);
		const form = setup(save);
		await form.user.type(form.content(), 'x');
		await form.submit();
		expect(screen.getByRole('alert')).toHaveTextContent("Couldn't save the memory");
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(form.onsaved).not.toHaveBeenCalled();
	});

	it('disables Save while saving', async () => {
		let finish!: (result: Result<Memory>) => void;
		const save = vi.fn(() => new Promise<Result<Memory>>((resolve) => (finish = resolve)));
		const form = setup(save);
		await form.user.type(form.content(), 'x');
		await form.submit();
		expect(screen.getByRole('button', { name: 'Saving…' })).toBeDisabled();
		finish({ kind: 'ok', data: stored });
		await waitFor(() => expect(form.onsaved).toHaveBeenCalledTimes(1));
	});
});
