import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type Goal, type GoalDetail } from '#lib/api';
import GoalsView from '#lib/components/goals/GoalsView.svelte';
import type { Result } from './api';
import { GoalsStore } from './goals.svelte';

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function makeGoal(id: number, overrides: Partial<Goal> = {}): Goal {
	return {
		id,
		text: `Goal ${id}`,
		status: 'planning',
		error: null,
		auto_replans: 0,
		current_plan_id: null,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		...overrides
	};
}

function detail(id: number): GoalDetail {
	return { ...makeGoal(id), questions: [], plan: null, versions: [], run: null };
}

/** A store whose `fetch` answers each request with the next of `answers`. */
function setup(...answers: Response[]) {
	const fetch = vi.fn<typeof globalThis.fetch>(async () => answers.shift() ?? json([], 500));
	const navigate = vi.fn();
	const store = new GoalsStore({
		client: createApiClient({ baseUrl: 'http://api.test', fetch }),
		navigate
	});
	return { store, fetch, navigate };
}

describe('GoalsStore', () => {
	it('loads the list, newest first as the API sends it', async () => {
		const { store, fetch } = setup(json([makeGoal(2), makeGoal(1)]));
		expect(store.view.kind).toBe('loading');
		await store.load();
		expect(store.view.kind).toBe('loaded');
		expect(store.goals.map((goal) => goal.id)).toEqual([2, 1]);
		expect(new URL((fetch.mock.calls[0][0] as Request).url).pathname).toBe('/goals');
	});

	it('shows the error state when the first load fails, and keeps the list on a later failure', async () => {
		const { store } = setup(json({}, 500), json([makeGoal(1)]), json({}, 500));
		await store.load();
		expect(store.view.kind).toBe('error');
		await store.load();
		expect(store.view.kind).toBe('loaded');
		await store.load();
		expect(store.view.kind).toBe('loaded');
		expect(store.goals).toHaveLength(1);
	});

	it('creates a goal with the trimmed text and opens its page', async () => {
		const { store, fetch, navigate } = setup(json([]), json(detail(7), 201));
		await store.load();
		expect(await store.create('  Frontend roles in NYC  ')).toBe(true);
		const request = fetch.mock.calls[1][0] as Request;
		expect(request.method).toBe('POST');
		expect(new URL(request.url).pathname).toBe('/goals');
		expect(await request.clone().json()).toEqual({ text: 'Frontend roles in NYC' });
		expect(navigate).toHaveBeenCalledWith(7);
		expect(store.goals.map((goal) => goal.id)).toEqual([7]);
		expect(store.error).toBeNull();
	});

	it('shows a 422 message and stays on the page', async () => {
		const { store, navigate } = setup(
			json({ errors: [{ field: 'text', message: 'At most 1000 characters' }] }, 422)
		);
		expect(await store.create('x')).toBe(false);
		expect(store.error).toBe('At most 1000 characters');
		expect(navigate).not.toHaveBeenCalled();
	});

	it('sends nothing for a blank goal', async () => {
		const { store, fetch } = setup();
		expect(await store.create('   ')).toBe(false);
		expect(fetch).not.toHaveBeenCalled();
	});

	it('reloads on goal.updated and after a reconnect', async () => {
		const list = vi.fn<() => Promise<Result<Goal[]>>>(async () => ({
			kind: 'ok',
			data: [makeGoal(1)]
		}));
		const store = new GoalsStore({ list, navigate: vi.fn() });
		await store.load();
		store.handle({ type: 'goal.updated', goal: makeGoal(1, { status: 'planned' }) });
		store.handle({ type: 'app.connected' });
		store.handle({ type: 'queue.resumed' });
		expect(list).toHaveBeenCalledTimes(3);
	});
});

describe('GoalsView', () => {
	const props = {
		goals: [] as Goal[],
		oncreate: vi.fn(async () => true),
		onreload: vi.fn()
	};

	it('shows a skeleton while loading', () => {
		render(GoalsView, { ...props, view: { kind: 'loading' } });
		expect(screen.getByLabelText('Loading goals')).toHaveAttribute('aria-busy', 'true');
	});

	it('shows the empty state', () => {
		render(GoalsView, { ...props, view: { kind: 'loaded' } });
		expect(screen.getByText('No goals yet')).toBeInTheDocument();
	});

	it('shows the error state with Try again', async () => {
		const onreload = vi.fn();
		render(GoalsView, { ...props, onreload, view: { kind: 'error' } });
		expect(screen.getByText("Couldn't load your goals")).toBeInTheDocument();
		await userEvent.setup().click(screen.getByRole('button', { name: 'Try again' }));
		expect(onreload).toHaveBeenCalledOnce();
	});

	it('lists goals with their status, linking to their pages', () => {
		render(GoalsView, {
			...props,
			view: { kind: 'loaded' },
			goals: [makeGoal(3, { text: 'Climate roles', status: 'needs_answers' })]
		});
		const link = screen.getByRole('link', { name: /Climate roles/ });
		expect(link).toHaveAttribute('href', '/goals/3');
		expect(link).toHaveTextContent('Needs answers');
	});

	it('keeps Plan it disabled until there is text, then sends it', async () => {
		const oncreate = vi.fn(async () => true);
		render(GoalsView, { ...props, oncreate, view: { kind: 'loaded' } });
		const button = screen.getByRole('button', { name: 'Plan it' });
		expect(button).toBeDisabled();
		const user = userEvent.setup();
		await user.type(screen.getByLabelText('What are you looking for?'), 'Rust roles');
		expect(button).toBeEnabled();
		await user.click(button);
		expect(oncreate).toHaveBeenCalledWith('Rust roles');
	});

	it('shows the error under the field and disables the button while creating', () => {
		render(GoalsView, {
			...props,
			view: { kind: 'loaded' },
			creating: true,
			error: 'At most 1000 characters'
		});
		expect(screen.getByLabelText('What are you looking for?')).toHaveAccessibleDescription(
			expect.stringContaining('At most 1000 characters')
		);
		expect(screen.getByRole('button', { name: 'Planning…' })).toBeDisabled();
	});
});
