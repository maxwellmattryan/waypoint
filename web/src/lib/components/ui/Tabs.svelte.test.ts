import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import TabsHarness from './__fixtures__/TabsHarness.svelte';

const items = [
	{ value: 'overview', label: 'Overview' },
	{ value: 'notes', label: 'Notes' },
	{ value: 'archived', label: 'Archived', disabled: true },
	{ value: 'emails', label: 'Emails' }
];

function tab(name: string) {
	return screen.getByRole('tab', { name });
}

async function expectSelected(name: string, value: string) {
	const selected = tab(name);
	expect(selected).toHaveAttribute('aria-selected', 'true');
	expect(selected).toHaveAttribute('tabindex', '0');
	expect(selected).toHaveFocus();
	expect(screen.getByTestId('value')).toHaveTextContent(value);
	await waitFor(() => expect(screen.getByRole('tabpanel')).toHaveTextContent(`Panel ${value}`));
}

describe('Tabs', () => {
	it('renders a tablist with tabs, a tabpanel and a roving tabindex', () => {
		render(TabsHarness, { items });
		expect(screen.getByRole('tablist', { name: 'Application' })).toBeInTheDocument();
		const tabs = screen.getAllByRole('tab');
		expect(tabs).toHaveLength(4);
		const panel = screen.getByRole('tabpanel');

		expect(tabs.map((t) => t.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false', 'false']);
		expect(tabs.map((t) => t.getAttribute('tabindex'))).toEqual(['0', '-1', '-1', '-1']);
		for (const t of tabs) expect(t).toHaveAttribute('aria-controls', panel.id);
		expect(panel).toHaveAttribute('aria-labelledby', tabs[0].id);
		expect(panel).toHaveAttribute('tabindex', '0');
		expect(panel).toHaveTextContent('Panel overview');
		expect(screen.getByTestId('value')).toHaveTextContent('overview');
	});

	it('defaults to the first enabled tab', () => {
		render(TabsHarness, { items: [{ ...items[0], disabled: true }, items[1]] });
		expect(tab('Notes')).toHaveAttribute('aria-selected', 'true');
		expect(screen.getByTestId('value')).toHaveTextContent('notes');
	});

	it('selects an enabled tab on click and ignores a disabled one', async () => {
		const user = userEvent.setup();
		render(TabsHarness, { items });
		await user.click(tab('Notes'));
		await expectSelected('Notes', 'notes');
		expect(screen.getByRole('tabpanel')).toHaveAttribute('aria-labelledby', tab('Notes').id);

		await user.click(tab('Archived'));
		expect(tab('Archived')).toHaveAttribute('aria-selected', 'false');
		expect(screen.getByTestId('value')).toHaveTextContent('notes');
	});

	it('moves with arrows, skipping disabled tabs and wrapping, and Home/End', async () => {
		const user = userEvent.setup();
		render(TabsHarness, { items, value: 'notes' });
		tab('Notes').focus();

		await user.keyboard('{ArrowRight}');
		await expectSelected('Emails', 'emails');
		await user.keyboard('{ArrowRight}');
		await expectSelected('Overview', 'overview');
		await user.keyboard('{ArrowLeft}');
		await expectSelected('Emails', 'emails');
		await user.keyboard('{ArrowLeft}');
		await expectSelected('Notes', 'notes');
		await user.keyboard('{Home}');
		await expectSelected('Overview', 'overview');
		await user.keyboard('{End}');
		await expectSelected('Emails', 'emails');
	});
});
