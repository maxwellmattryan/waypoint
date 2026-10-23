import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { PendingAction } from '#lib/api';
import type { Result } from '#lib/applications/api.js';
import EditActionDialog from './EditActionDialog.svelte';

type Approve = (input: Record<string, unknown>) => Promise<Result<PendingAction>>;

function setup(onapprove: Approve) {
	const user = userEvent.setup();
	render(EditActionDialog, { open: true, input: { message: 'Thanks' }, onapprove });
	const field = () => screen.getByLabelText('Input (JSON)');
	return {
		user,
		field,
		/** Replaces the field's text. `paste` avoids `type`'s special `{` and `[` keys. */
		async enter(text: string) {
			await user.clear(field());
			await user.click(field());
			await user.paste(text);
		},
		submit: () => user.click(screen.getByRole('button', { name: 'Approve edited' }))
	};
}

describe('EditActionDialog', () => {
	it('prefills the requested input as pretty JSON', () => {
		setup(vi.fn<Approve>());
		expect(screen.getByRole('dialog', { name: 'Edit before approving' })).toBeInTheDocument();
		expect(screen.getByLabelText('Input (JSON)')).toHaveValue('{\n  "message": "Thanks"\n}');
	});

	it.each([
		['{"message": ', 'Not valid JSON'],
		['[]', 'Must be a JSON object'],
		['null', 'Must be a JSON object'],
		['"text"', 'Must be a JSON object']
	])('refuses %s with "%s" and does not approve', async (text, message) => {
		const onapprove = vi.fn<Approve>();
		const { enter, submit, field } = setup(onapprove);
		await enter(text);
		await submit();
		expect(screen.getByText(message)).toBeInTheDocument();
		expect(field()).toHaveAttribute('aria-invalid', 'true');
		expect(onapprove).not.toHaveBeenCalled();
	});

	it('approves a valid object and closes', async () => {
		const onapprove = vi.fn<Approve>().mockResolvedValue({ kind: 'ok', data: {} as PendingAction });
		const { enter, submit } = setup(onapprove);
		await enter('{"message": "Edited"}');
		await submit();
		expect(onapprove).toHaveBeenCalledWith({ message: 'Edited' });
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('shows a 422 message under the field and stays open with the text kept', async () => {
		const onapprove = vi.fn<Approve>().mockResolvedValue({
			kind: 'invalid',
			errors: [{ field: 'input', message: '/message: must be a string' }]
		});
		const { enter, submit, field } = setup(onapprove);
		await enter('{"message": 1}');
		await submit();
		expect(await screen.findByText('/message: must be a string')).toBeInTheDocument();
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(field()).toHaveValue('{"message": 1}');
	});
});
