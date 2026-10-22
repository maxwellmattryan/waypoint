import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Button, IconButton } from './index';
import { text } from './__fixtures__/snippets';

describe('Button', () => {
	it('calls onclick when clicked', async () => {
		const onclick = vi.fn();
		render(Button, { onclick, children: text('Save') });
		await userEvent.click(screen.getByRole('button', { name: 'Save' }));
		expect(onclick).toHaveBeenCalledTimes(1);
	});

	it('does not call onclick when disabled', async () => {
		const onclick = vi.fn();
		render(Button, { onclick, disabled: true, children: text('Save') });
		const button = screen.getByRole('button', { name: 'Save' });
		expect(button).toBeDisabled();
		await userEvent.click(button);
		expect(onclick).not.toHaveBeenCalled();
	});

	it('defaults type to "button" and accepts an override', () => {
		render(Button, { children: text('Plain') });
		render(Button, { type: 'submit', children: text('Send') });
		expect(screen.getByRole('button', { name: 'Plain' })).toHaveAttribute('type', 'button');
		expect(screen.getByRole('button', { name: 'Send' })).toHaveAttribute('type', 'submit');
	});

	it('merges the caller class after its own and spreads attributes', () => {
		render(Button, { class: 'w-full', 'data-testid': 'b', children: text('Wide') });
		const button = screen.getByTestId('b');
		expect(button.className.endsWith('w-full')).toBe(true);
	});
});

describe('IconButton', () => {
	it('uses its label as the accessible name', async () => {
		const onclick = vi.fn();
		render(IconButton, { label: 'Close', onclick, children: text('x') });
		const button = screen.getByRole('button', { name: 'Close' });
		expect(button).toHaveAttribute('type', 'button');
		await userEvent.click(button);
		expect(onclick).toHaveBeenCalledTimes(1);
	});
});
