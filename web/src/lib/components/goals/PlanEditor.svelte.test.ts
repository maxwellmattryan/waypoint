import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { GoalDetail, Plan, PlanEdit } from '#lib/api';
import type { GoalResult } from '#lib/goals/api.js';
import { makeDetail, makePlan } from '#lib/goals/__fixtures__/goals.js';
import PlanEditor from './PlanEditor.svelte';

type Save = (body: PlanEdit) => Promise<GoalResult<GoalDetail>>;

function setup(answer: Awaited<ReturnType<Save>> = { kind: 'ok', data: makeDetail() }, plan: Plan = makePlan()) {
	const onsave = vi.fn<Save>(async () => answer);
	const view = render(PlanEditor, { open: true, plan, onsave });
	return { onsave, view, user: userEvent.setup() };
}

const companies = () => within(screen.getByRole('list', { name: 'Companies' }));
const save = () => screen.getByRole('button', { name: 'Save plan' });
const addButton = () => screen.getByRole('button', { name: 'Add company' });

async function fillCompany(
	user: ReturnType<typeof userEvent.setup>,
	name: string,
	source: string,
	board: string
) {
	if (name) await user.type(screen.getByLabelText('Company name'), name);
	if (source) await user.selectOptions(screen.getByLabelText('Board'), source);
	if (board) await user.type(screen.getByLabelText('Board slug'), board);
	await user.click(addButton());
}

describe('PlanEditor', () => {
	it('sends base_plan_id with the form values', async () => {
		const { onsave, user } = setup();
		const min = screen.getByLabelText('Minimum score');
		await user.clear(min);
		await user.type(min, '40');
		await user.click(screen.getByLabelText('Remote only'));
		await user.click(save());
		expect(onsave).toHaveBeenCalledWith({
			base_plan_id: 11,
			companies: [
				{
					name: 'Example Labs',
					source: 'greenhouse',
					board: 'example-labs',
					reason: 'Builds grid software'
				},
				{ name: 'Watershed', source: 'ashby', board: 'watershed', reason: null }
			],
			title_keywords: ['frontend'],
			locations: ['new york'],
			remote_only: true,
			max_to_score: 20,
			shortlist_size: 5,
			min_score: 40,
			prep_notes: 'Lead with design systems'
		});
	});

	it('adds and removes companies', async () => {
		const { onsave, user } = setup();
		await fillCompany(user, 'Arcadia', 'greenhouse', 'arcadia');
		expect(companies().getByText('greenhouse/arcadia')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Remove Watershed' }));
		expect(companies().queryByText('ashby/watershed')).not.toBeInTheDocument();
		await user.click(save());
		const sent = onsave.mock.calls[0][0];
		expect(sent.companies?.map((c) => c.board)).toEqual(['example-labs', 'arcadia']);
	});

	it('validates the source and slug like the route, and refuses a duplicate board', async () => {
		const { user } = setup();
		await fillCompany(user, '', '', 'bad slug');
		expect(screen.getByLabelText('Company name')).toHaveAccessibleDescription('Required');
		expect(screen.getByLabelText('Board')).toHaveAccessibleDescription('Required');
		expect(screen.getByLabelText('Board slug')).toHaveAccessibleDescription(
			'Must be 1–100 letters, digits, hyphens or underscores'
		);
		await user.clear(screen.getByLabelText('Board slug'));
		await user.type(screen.getByLabelText('Company name'), 'Again');
		await user.selectOptions(screen.getByLabelText('Board'), 'greenhouse');
		await user.type(screen.getByLabelText('Board slug'), 'example-labs');
		await user.click(addButton());
		expect(screen.getByLabelText('Board slug')).toHaveAccessibleDescription('Duplicate board');
		expect(companies().getAllByRole('listitem')).toHaveLength(2);
	});

	it('edits a company in place', async () => {
		const { onsave, user } = setup();
		await user.click(screen.getByRole('button', { name: 'Edit Watershed' }));
		const name = screen.getByLabelText('Company name');
		await user.clear(name);
		await user.type(name, 'Watershed Climate');
		await user.click(screen.getByRole('button', { name: 'Update company' }));
		await user.click(save());
		expect(onsave.mock.calls[0][0].companies?.[1]?.name).toBe('Watershed Climate');
	});

	it('checks the numbers before sending', async () => {
		const { onsave, user } = setup();
		const size = screen.getByLabelText('Shortlist size');
		await user.clear(size);
		await user.type(size, '0');
		await user.click(save());
		expect(size).toHaveAccessibleDescription(expect.stringContaining('Must be 1 to 20'));
		expect(onsave).not.toHaveBeenCalled();
	});

	it('shows the stale-plan message on 409', async () => {
		const { user } = setup({ kind: 'conflict', message: 'This plan changed since you opened it' });
		await user.click(save());
		expect(await screen.findByRole('alert')).toHaveTextContent(
			'This plan changed since you opened it'
		);
		expect(save()).toBeEnabled();
	});

	it('maps a 422 onto the fields', async () => {
		const { user } = setup({
			kind: 'invalid',
			errors: [
				{ field: 'companies[1].board', message: 'Duplicate board' },
				{ field: 'max_to_score', message: 'Must be 1 to 50' }
			]
		});
		await user.click(save());
		expect(await companies().findByText('Duplicate board')).toBeInTheDocument();
		expect(screen.getByLabelText('Most postings to score')).toHaveAccessibleDescription(
			expect.stringContaining('Must be 1 to 50')
		);
	});
});
