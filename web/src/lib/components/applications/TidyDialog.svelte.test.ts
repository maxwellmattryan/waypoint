import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Run, TidyRequest } from '#lib/api';
import type { Result } from '#lib/applications/api.js';
import { toast } from '#lib/components/ui';
import TidyDialog from './TidyDialog.svelte';

type Start = (body: TidyRequest) => Promise<Result<Run>>;

afterEach(() => toast.clear());

function setup(onstart: Start) {
	const user = userEvent.setup();
	render(TidyDialog, { open: true, onstart });
	return {
		user,
		note: () => screen.getByLabelText('Note'),
		message: () => screen.getByLabelText('Follow-up message'),
		confidence: () => screen.getByLabelText('Confidence'),
		submit: () => user.click(screen.getByRole('button', { name: 'Start run' }))
	};
}

describe('TidyDialog', () => {
	it('defaults confidence to 1 with its hint', () => {
		const { confidence } = setup(vi.fn<Start>());
		expect(screen.getByRole('dialog', { name: 'Run sample workflow' })).toBeInTheDocument();
		expect(confidence()).toHaveValue(1);
		expect(
			screen.getByText('Below 0.8, the note goes to Review instead of the timeline.')
		).toBeInTheDocument();
	});

	it('refuses blank fields and a confidence outside 0 to 1 without sending', async () => {
		const onstart = vi.fn<Start>();
		const { user, note, message, confidence, submit } = setup(onstart);
		await user.type(note(), '   ');
		await user.clear(confidence());
		await user.type(confidence(), '1.5');
		await submit();
		expect(screen.getAllByText('Required')).toHaveLength(2);
		expect(note()).toHaveAttribute('aria-invalid', 'true');
		expect(message()).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByText('Enter a number from 0 to 1')).toBeInTheDocument();
		expect(confidence()).toHaveAttribute('aria-invalid', 'true');

		await user.clear(confidence());
		await submit();
		expect(screen.getByText('Enter a number from 0 to 1')).toBeInTheDocument();
		expect(onstart).not.toHaveBeenCalled();
	});

	it('sends trimmed values with a numeric confidence, then closes and toasts', async () => {
		const onstart = vi.fn<Start>().mockResolvedValue({ kind: 'ok', data: { id: 42 } as Run });
		const { user, note, message, confidence, submit } = setup(onstart);
		await user.type(note(), '  Ask about the team ');
		await user.type(message(), ' Thanks ');
		await user.clear(confidence());
		await user.type(confidence(), '0.5');
		await submit();
		expect(onstart).toHaveBeenCalledWith({
			note: 'Ask about the team',
			message: 'Thanks',
			confidence: 0.5
		});
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
		expect(toast.items.map((t) => [t.tone, t.title, t.description])).toEqual([
			['success', 'Run #42 started', 'Its draft will wait for your approval in Review.']
		]);
	});

	it('shows a 422 for confidence under that field and stays open', async () => {
		const onstart = vi.fn<Start>().mockResolvedValue({
			kind: 'invalid',
			errors: [{ field: 'confidence', message: 'must be from 0 to 1' }]
		});
		const { user, note, message, submit, confidence } = setup(onstart);
		await user.type(note(), 'n');
		await user.type(message(), 'm');
		await submit();
		expect(await screen.findByText('must be from 0 to 1')).toBeInTheDocument();
		expect(confidence()).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByRole('dialog')).toBeInTheDocument();
	});

	it('keeps the dialog open and toasts when the request fails', async () => {
		const onstart = vi.fn<Start>().mockResolvedValue({ kind: 'failed' });
		const { user, note, message, submit } = setup(onstart);
		await user.type(note(), 'n');
		await user.type(message(), 'm');
		await submit();
		await waitFor(() =>
			expect(toast.items.map((t) => t.title)).toEqual(["Couldn't start the run"])
		);
		expect(screen.getByRole('dialog')).toBeInTheDocument();
	});
});
