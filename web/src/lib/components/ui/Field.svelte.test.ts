import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import type { Component } from 'svelte';
import { describe, expect, it } from 'vitest';
import { Input, Select, Textarea } from './index';

const options = [
	{ value: 'applied', label: 'Applied' },
	{ value: 'offer', label: 'Offer' }
];

type Control = {
	name: string;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	component: Component<any>;
	extra: Record<string, unknown>;
	/** Change the control's value through the UI and return the value expected in the binding. */
	change: (el: HTMLElement) => Promise<string>;
};

const controls: Control[] = [
	{
		name: 'Input',
		component: Input,
		extra: {},
		change: async (el) => {
			await userEvent.type(el, 'Acme');
			return 'Acme';
		}
	},
	{
		name: 'Textarea',
		component: Textarea,
		extra: {},
		change: async (el) => {
			await userEvent.type(el, 'Notes');
			return 'Notes';
		}
	},
	{
		name: 'Select',
		component: Select,
		extra: { options },
		change: async (el) => {
			await userEvent.selectOptions(el, 'offer');
			return 'offer';
		}
	}
];

describe.each(controls)('$name in a Field', ({ component, extra, change }) => {
	it('is found by its label and updates the bound value', async () => {
		let value: string | undefined = '';
		render(component, {
			label: 'Company',
			...extra,
			get value() {
				return value;
			},
			set value(next: string | undefined) {
				value = next;
			}
		});
		const control = screen.getByLabelText('Company');
		const expected = await change(control);
		expect(value).toBe(expected);
	});

	it('marks an error as invalid and describes the control with it', () => {
		render(component, { label: 'Company', error: 'Required', ...extra });
		const control = screen.getByLabelText('Company');
		expect(control).toHaveAttribute('aria-invalid', 'true');
		expect(control).toHaveAccessibleDescription('Required');
	});

	it('links the hint through aria-describedby', () => {
		render(component, { label: 'Company', hint: 'As on the posting', ...extra });
		const control = screen.getByLabelText('Company');
		expect(control).not.toHaveAttribute('aria-invalid');
		expect(control).toHaveAccessibleDescription('As on the posting');
	});
});

describe('Select', () => {
	it('renders the placeholder as a disabled, empty first option', () => {
		render(Select, { label: 'Stage', options, placeholder: 'Pick a stage' });
		const select = screen.getByLabelText('Stage') as HTMLSelectElement;
		const first = select.options[0];
		expect(first.textContent).toBe('Pick a stage');
		expect(first.value).toBe('');
		expect(first.disabled).toBe(true);
		expect(select.value).toBe('');
	});
});
