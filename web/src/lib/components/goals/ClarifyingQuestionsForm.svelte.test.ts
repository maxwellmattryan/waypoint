import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { makeQuestion } from '#lib/goals/__fixtures__/goals.js';
import ClarifyingQuestionsForm from './ClarifyingQuestionsForm.svelte';

function setup(busy = false) {
	const onsubmit = vi.fn();
	render(ClarifyingQuestionsForm, {
		questions: [makeQuestion(4, { question: 'Which roles?' }), makeQuestion(5, { question: 'Where?' })],
		busy,
		onsubmit
	});
	return { onsubmit, user: userEvent.setup() };
}

describe('ClarifyingQuestionsForm', () => {
	it('sends each trimmed answer, a blank one as skipped', async () => {
		const { onsubmit, user } = setup();
		await user.type(screen.getByLabelText('Which roles?'), '  Senior frontend  ');
		await user.click(screen.getByRole('button', { name: 'Submit answers' }));
		expect(onsubmit).toHaveBeenCalledWith([
			{ question_id: 4, answer: 'Senior frontend' },
			{ question_id: 5 }
		]);
	});

	it('skips every question with Skip and plan anyway', async () => {
		const { onsubmit, user } = setup();
		await user.type(screen.getByLabelText('Where?'), 'NYC');
		await user.click(screen.getByRole('button', { name: 'Skip and plan anyway' }));
		expect(onsubmit).toHaveBeenCalledWith([{ question_id: 4 }, { question_id: 5 }]);
	});

	it('refuses an answer over 1000 characters', async () => {
		const { onsubmit, user } = setup();
		const field = screen.getByLabelText('Where?');
		await user.click(field);
		await user.paste('x'.repeat(1001));
		expect(field).toHaveAccessibleDescription(expect.stringContaining('At most 1000 characters'));
		expect(screen.getByRole('button', { name: 'Submit answers' })).toBeDisabled();
		expect(onsubmit).not.toHaveBeenCalled();
	});

	it('disables both buttons while sending', () => {
		setup(true);
		expect(screen.getByRole('button', { name: 'Sending…' })).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Skip and plan anyway' })).toBeDisabled();
	});
});
