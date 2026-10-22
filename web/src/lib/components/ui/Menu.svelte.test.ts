import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { durations } from '#lib/motion.js';
import MenuHarness from './__fixtures__/MenuHarness.svelte';

afterEach(() => vi.restoreAllMocks());

function trigger() {
	return screen.getByRole('button', { name: 'Move' });
}

function item(name: string) {
	return screen.getByRole('menuitem', { name });
}

async function expectClosed() {
	await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
	expect(trigger()).toHaveAttribute('aria-expanded', 'false');
	expect(trigger()).not.toHaveAttribute('aria-controls');
}

function expectOpen() {
	const menu = screen.getByRole('menu', { name: 'Move to' });
	expect(trigger()).toHaveAttribute('aria-expanded', 'true');
	expect(trigger()).toHaveAttribute('aria-controls', menu.id);
	return menu;
}

describe('Menu', () => {
	it('is closed at first, with a menu-button trigger', () => {
		render(MenuHarness);
		expect(screen.queryByRole('menu')).toBeNull();
		expect(trigger()).toHaveAttribute('aria-haspopup', 'menu');
		expect(trigger()).toHaveAttribute('aria-expanded', 'false');
	});

	it.each([
		['click', null],
		['Enter', '{Enter}'],
		['Space', ' '],
		['ArrowDown', '{ArrowDown}']
	])('%s opens it and focuses the first item', async (_, keys) => {
		const user = userEvent.setup();
		render(MenuHarness);
		if (keys === null) await user.click(trigger());
		else {
			trigger().focus();
			await user.keyboard(keys);
		}
		const menu = expectOpen();
		expect(item('Applied')).toHaveFocus();
		// The heading is text, not an item; the items keep their order.
		expect(menu).toHaveTextContent(/^Move to/);
		expect(screen.getAllByRole('menuitem').map((el) => el.textContent?.trim())).toEqual([
			'Applied',
			'Offer',
			'Interviewing',
			'Rejected'
		]);
	});

	it('ArrowUp opens it and focuses the last item', async () => {
		const user = userEvent.setup();
		render(MenuHarness);
		trigger().focus();
		await user.keyboard('{ArrowUp}');
		expectOpen();
		expect(item('Rejected')).toHaveFocus();
	});

	it('arrows wrap, Home and End jump, and disabled items are skipped', async () => {
		const user = userEvent.setup();
		render(MenuHarness);
		await user.click(trigger());
		expect(item('Applied')).toHaveFocus();
		await user.keyboard('{ArrowDown}');
		expect(item('Interviewing')).toHaveFocus();
		await user.keyboard('{ArrowDown}');
		expect(item('Rejected')).toHaveFocus();
		await user.keyboard('{ArrowDown}');
		expect(item('Applied')).toHaveFocus();
		await user.keyboard('{ArrowUp}');
		expect(item('Rejected')).toHaveFocus();
		await user.keyboard('{ArrowUp}');
		expect(item('Interviewing')).toHaveFocus();
		await user.keyboard('{ArrowUp}');
		expect(item('Applied')).toHaveFocus();
		await user.keyboard('{End}');
		expect(item('Rejected')).toHaveFocus();
		await user.keyboard('{Home}');
		expect(item('Applied')).toHaveFocus();
		expect(item('Offer')).toBeDisabled();
	});

	it('selecting an item calls onselect, closes and refocuses the trigger', async () => {
		const user = userEvent.setup();
		const onselect = vi.fn();
		render(MenuHarness, { onselect });
		await user.click(trigger());
		await user.click(item('Interviewing'));
		expect(onselect).toHaveBeenCalledExactlyOnceWith('interviewing');
		await expectClosed();
		expect(trigger()).toHaveFocus();

		// By keyboard, too.
		trigger().focus();
		await user.keyboard('{ArrowDown}{ArrowDown}{Enter}');
		expect(onselect).toHaveBeenLastCalledWith('interviewing');
		await expectClosed();
		expect(trigger()).toHaveFocus();

		// A disabled item does nothing.
		await user.click(trigger());
		await fireEvent.click(item('Offer'));
		expect(onselect).toHaveBeenCalledTimes(2);
		expectOpen();
	});

	it('Escape closes it, refocuses the trigger and keeps a Dialog behind it open', async () => {
		const user = userEvent.setup();
		const onWindowKeydown = vi.fn();
		window.addEventListener('keydown', onWindowKeydown);
		render(MenuHarness, { inDialog: true });
		await user.click(trigger());
		expectOpen();
		await user.keyboard('{Escape}');
		await expectClosed();
		expect(trigger()).toHaveFocus();
		expect(onWindowKeydown).not.toHaveBeenCalled();
		expect(screen.getByTestId('dialog-state')).toHaveAttribute('data-open', 'true');
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		window.removeEventListener('keydown', onWindowKeydown);
	});

	it('Tab and a pointer-down outside close it', async () => {
		const user = userEvent.setup();
		render(MenuHarness);
		await user.click(trigger());
		expectOpen();
		await user.keyboard('{Tab}');
		await expectClosed();

		await user.click(trigger());
		expectOpen();
		await fireEvent.pointerDown(screen.getByRole('button', { name: 'Outside' }));
		await expectClosed();

		// A pointer-down inside it doesn't.
		await user.click(trigger());
		await fireEvent.pointerDown(item('Applied'));
		expectOpen();
	});

	it('fades with the fast motion token', async () => {
		const animate = vi.spyOn(Element.prototype, 'animate');
		const user = userEvent.setup();
		render(MenuHarness);
		await user.click(trigger());
		const menu = expectOpen();
		// Svelte may start with a zero-length frame; the transition itself runs for `fast`.
		const durationsOnMenu = animate.mock.calls
			.filter((_, i) => animate.mock.contexts[i] === menu)
			.map(([, options]) => (typeof options === 'number' ? options : options?.duration));
		expect(durationsOnMenu).toContain(durations.fast);
	});
});
