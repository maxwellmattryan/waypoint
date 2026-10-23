import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import TagInputHarness from './__fixtures__/TagInputHarness.svelte';
import { Tag, TagInput } from './index';

function tags(): string[] {
	return JSON.parse(screen.getByTestId('tags').textContent ?? '[]');
}

function setup(props: { initial?: string[]; max?: number } = {}) {
	const user = userEvent.setup();
	const onsubmit = vi.fn();
	render(TagInputHarness, { ...props, onsubmit });
	return { user, onsubmit, input: screen.getByLabelText('Tags') };
}

describe('Tag', () => {
	it('is a plain span without handlers', () => {
		const { container } = render(Tag, { label: 'remote' });
		expect(container.querySelector('button')).toBeNull();
		expect(screen.getByText('remote').closest('span')).not.toBeNull();
	});

	it('is a button with onclick', async () => {
		const onclick = vi.fn();
		render(Tag, { label: 'remote', onclick });
		await userEvent.click(screen.getByRole('button', { name: 'remote' }));
		expect(onclick).toHaveBeenCalledTimes(1);
	});

	it('has a working remove button with onremove', async () => {
		const onremove = vi.fn();
		const onclick = vi.fn();
		render(Tag, { label: 'remote', onremove, onclick });
		await userEvent.click(screen.getByRole('button', { name: 'Remove tag remote' }));
		expect(onremove).toHaveBeenCalledTimes(1);
		expect(onclick).not.toHaveBeenCalled();
	});
});

describe('TagInput', () => {
	it('adds a normalised tag on Enter and on comma', async () => {
		const { user, input } = setup();
		await user.type(input, '  Remote   First {Enter}');
		expect(tags()).toEqual(['remote first']);
		expect(input).toHaveValue('');
		await user.type(input, 'Rust,');
		expect(tags()).toEqual(['remote first', 'rust']);
		expect(input).toHaveValue('');
		expect(screen.getByRole('button', { name: 'Remove tag rust' })).toBeInTheDocument();
	});

	it("doesn't add a duplicate or a blank tag", async () => {
		const { user, input } = setup({ initial: ['rust'] });
		await user.type(input, 'RUST{Enter}');
		await user.type(input, '   {Enter}');
		expect(tags()).toEqual(['rust']);
	});

	it('removes the last tag on Backspace in an empty input', async () => {
		const { user, input } = setup({ initial: ['go', 'rust'] });
		await user.type(input, 'x{Backspace}');
		expect(tags()).toEqual(['go', 'rust']);
		await user.keyboard('{Backspace}');
		expect(tags()).toEqual(['go']);
	});

	it('removes a tag with its remove button', async () => {
		const { user, input } = setup({ initial: ['go', 'rust', 'svelte'] });
		await user.click(screen.getByRole('button', { name: 'Remove tag rust' }));
		expect(tags()).toEqual(['go', 'svelte']);
		await waitFor(() =>
			expect(screen.queryByRole('button', { name: 'Remove tag rust' })).toBeNull()
		);
		expect(input).toHaveFocus();
	});

	it('stops adding at max and says so', async () => {
		const { user, input } = setup({ initial: ['a'], max: 2 });
		expect(input).not.toHaveAccessibleDescription();
		await user.type(input, 'b{Enter}c{Enter}');
		expect(tags()).toEqual(['a', 'b']);
		expect(input).toHaveValue('c');
		expect(input).toHaveAccessibleDescription('At most 2 tags');
	});

	it('never submits an enclosing form on Enter', async () => {
		const { user, input, onsubmit } = setup();
		await user.type(input, 'go{Enter}');
		await user.type(input, '{Enter}');
		expect(onsubmit).not.toHaveBeenCalled();
		expect(tags()).toEqual(['go']);
	});

	it('links its label, hint and error like Input', () => {
		render(TagInput, { label: 'Labels', hint: 'Press Enter to add', error: 'Too long' });
		const input = screen.getByLabelText('Labels');
		expect(input).toHaveAttribute('aria-invalid', 'true');
		expect(input).toHaveAccessibleDescription('Press Enter to add Too long');
	});
});
