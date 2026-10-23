import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import ConfirmDialogHarness from './__fixtures__/ConfirmDialogHarness.svelte';
import { ConfirmDialog } from './index';

/** A promise and the function that settles it. */
function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

function setup(onconfirm: () => Promise<boolean>) {
	render(ConfirmDialogHarness, { onconfirm });
	return {
		isOpen: () => screen.getByTestId('state').getAttribute('data-open') === 'true'
	};
}

describe('ConfirmDialog', () => {
	it('is a labelled dialog with its description', () => {
		setup(async () => true);
		const dialog = screen.getByRole('dialog', { name: 'Delete memory' });
		expect(dialog).toHaveAccessibleDescription("Waypoint will forget this. This can't be undone.");
	});

	it('shows the pending label and disables confirm while running, then closes on true', async () => {
		const result = deferred<boolean>();
		const onconfirm = vi.fn(() => result.promise);
		const { isOpen } = setup(onconfirm);
		await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
		expect(onconfirm).toHaveBeenCalledTimes(1);
		const pending = screen.getByRole('button', { name: 'Deleting…' });
		expect(pending).toBeDisabled();
		await userEvent.click(pending);
		expect(onconfirm).toHaveBeenCalledTimes(1);

		result.resolve(true);
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(isOpen()).toBe(false);
	});

	it('stays open with confirm enabled again on false', async () => {
		const onconfirm = vi.fn(async () => false);
		const { isOpen } = setup(onconfirm);
		await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
		await waitFor(() => expect(screen.getByRole('button', { name: 'Delete' })).toBeEnabled());
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(isOpen()).toBe(true);
	});

	it('closes on Cancel without calling onconfirm', async () => {
		const onconfirm = vi.fn(async () => true);
		const { isOpen } = setup(onconfirm);
		await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(onconfirm).not.toHaveBeenCalled();
		expect(isOpen()).toBe(false);
	});

	it('takes its own labels and tone', async () => {
		render(ConfirmDialog, {
			open: true,
			title: 'Archive',
			confirmLabel: 'Archive',
			pendingLabel: 'Archiving…',
			tone: 'primary',
			onconfirm: () => new Promise<boolean>(() => {})
		});
		await userEvent.click(screen.getByRole('button', { name: 'Archive' }));
		expect(screen.getByRole('button', { name: 'Archiving…' })).toBeDisabled();
	});
});
