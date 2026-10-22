import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { Checkbox } from './index';

function renderBound(props: Record<string, unknown> = {}) {
	const state = { checked: false };
	render(Checkbox, {
		label: 'Remote only',
		...props,
		get checked() {
			return state.checked;
		},
		set checked(next: boolean) {
			state.checked = next;
		}
	});
	return state;
}

describe('Checkbox', () => {
	it('toggles the bound value when its label is clicked', async () => {
		const state = renderBound();
		await userEvent.click(screen.getByText('Remote only'));
		expect(state.checked).toBe(true);
		expect(screen.getByRole('checkbox', { name: 'Remote only' })).toBeChecked();
		await userEvent.click(screen.getByText('Remote only'));
		expect(state.checked).toBe(false);
	});

	it('does not toggle when disabled', async () => {
		const state = renderBound({ disabled: true });
		await userEvent.click(screen.getByText('Remote only'));
		expect(state.checked).toBe(false);
		expect(screen.getByRole('checkbox', { name: 'Remote only' })).not.toBeChecked();
	});
});
