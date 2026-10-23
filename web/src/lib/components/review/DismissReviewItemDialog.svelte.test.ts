import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ReviewItem } from '#lib/api';
import type { Result } from '#lib/applications/api.js';
import DismissReviewItemDialog from './DismissReviewItemDialog.svelte';

type Dismiss = (reason?: string) => Promise<Result<ReviewItem>>;

const ok: Result<ReviewItem> = { kind: 'ok', data: {} as ReviewItem };

function setup(ondismiss: Dismiss, busy = false) {
	const user = userEvent.setup();
	const view = render(DismissReviewItemDialog, { open: true, busy, ondismiss });
	const field = () => screen.getByLabelText('Reason');
	return {
		user,
		view,
		field,
		confirm: () => user.click(screen.getByRole('button', { name: 'Dismiss' }))
	};
}

describe('DismissReviewItemDialog', () => {
	it('opens with an empty optional reason', () => {
		setup(vi.fn<Dismiss>());
		expect(screen.getByRole('dialog', { name: 'Dismiss this result?' })).toBeInTheDocument();
		expect(screen.getByLabelText('Reason')).toHaveValue('');
	});

	it.each([
		['  Not relevant  ', 'Not relevant'],
		['', undefined],
		['   ', undefined]
	])('sends %j as %j and closes on ok', async (text, reason) => {
		const ondismiss = vi.fn<Dismiss>().mockResolvedValue(ok);
		const { user, field, confirm } = setup(ondismiss);
		if (text) await user.type(field(), text);
		await confirm();
		expect(ondismiss).toHaveBeenCalledExactlyOnceWith(reason);
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('shows a 422 message under the field and stays open with the text kept', async () => {
		const ondismiss = vi.fn<Dismiss>().mockResolvedValue({
			kind: 'invalid',
			errors: [{ field: 'reason', message: 'At most 1000 characters' }]
		});
		const { user, field, confirm } = setup(ondismiss);
		await user.type(field(), 'Too long');
		await confirm();
		expect(await screen.findByText('At most 1000 characters')).toBeInTheDocument();
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(field()).toHaveAttribute('aria-invalid', 'true');
		expect(field()).toHaveValue('Too long');
	});

	it.each(['conflict', 'not-found', 'failed'] as const)('closes on %s', async (kind) => {
		const ondismiss = vi.fn<Dismiss>().mockResolvedValue({ kind });
		const { confirm } = setup(ondismiss);
		await confirm();
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('disables Dismiss while a dismissal is in flight', async () => {
		let finish!: (result: Result<ReviewItem>) => void;
		const ondismiss = vi
			.fn<Dismiss>()
			.mockReturnValue(new Promise((resolve) => (finish = resolve)));
		const { confirm } = setup(ondismiss);
		await confirm();
		expect(screen.getByRole('button', { name: 'Dismissing…' })).toBeDisabled();
		finish(ok);
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('disables Dismiss while busy and cancels without dismissing', async () => {
		const ondismiss = vi.fn<Dismiss>();
		const { user } = setup(ondismiss, true);
		expect(screen.getByRole('button', { name: 'Dismiss' })).toBeDisabled();
		await user.click(screen.getByRole('button', { name: 'Cancel' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
		expect(ondismiss).not.toHaveBeenCalled();
	});

	it('clears the reason each time it opens', async () => {
		const ondismiss = vi.fn<Dismiss>();
		const { user, view, field } = setup(ondismiss);
		await user.type(field(), 'Draft');
		await user.click(screen.getByRole('button', { name: 'Cancel' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
		await view.rerender({ open: true });
		expect(await screen.findByLabelText('Reason')).toHaveValue('');
	});
});
