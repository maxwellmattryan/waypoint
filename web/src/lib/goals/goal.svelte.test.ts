import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createApiClient, type GoalDetail } from '#lib/api';
import GoalView from '#lib/components/goals/GoalView.svelte';
import { toast } from '#lib/components/ui';
import { makeDetail, makeGoal, makePlan, makeQuestion, makeStep } from './__fixtures__/goals';
import type { Result } from './api';
import { GoalStore, type GoalState } from './goal.svelte';

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

const toasts = () => toast.items.map((t) => [t.tone, t.title]);

afterEach(() => toast.clear());

function view(state: GoalState, handlers: Partial<Record<string, unknown>> = {}) {
	const props = {
		view: state,
		onanswer: vi.fn(),
		onrun: vi.fn(),
		onreplan: vi.fn(async () => true),
		onsave: vi.fn(async () => ({ kind: 'failed' as const })),
		onreload: vi.fn(),
		...handlers
	};
	render(GoalView, props);
	return props;
}

const loaded = (goal: GoalDetail): GoalState => ({ kind: 'loaded', goal });

describe('GoalView', () => {
	it('shows a skeleton while loading', () => {
		view({ kind: 'loading' });
		expect(screen.getByLabelText('Loading goal')).toHaveAttribute('aria-busy', 'true');
	});

	it('shows the 404 state with a link back', () => {
		view({ kind: 'not-found' });
		expect(screen.getByText('No such goal')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'All goals' })).toHaveAttribute('href', '/goals');
	});

	it('shows the load error with Try again', async () => {
		const props = view({ kind: 'error' });
		expect(screen.getByText("Couldn't load this goal")).toBeInTheDocument();
		await userEvent.setup().click(screen.getByRole('button', { name: 'Try again' }));
		expect(props.onreload).toHaveBeenCalledOnce();
	});

	it('shows planning with a link to the run', () => {
		view(
			loaded(
				makeDetail({
					...makeGoal({ status: 'planning', current_plan_id: null }),
					plan: null,
					run: { id: 8, status: 'running', error: null, created_at: '2026-10-22T00:00:00Z' }
				})
			)
		);
		expect(screen.getByRole('status')).toHaveTextContent('Planning…');
		expect(screen.getByRole('link', { name: 'Run #8' })).toHaveAttribute('href', '/runs/8');
		expect(screen.queryByRole('heading', { name: 'Plan' })).not.toBeInTheDocument();
	});

	it('shows a paused planner run', () => {
		view(
			loaded(
				makeDetail({
					...makeGoal({ status: 'planning', current_plan_id: null }),
					plan: null,
					run: { id: 8, status: 'paused', error: null, created_at: '2026-10-22T00:00:00Z' }
				})
			)
		);
		expect(screen.getByRole('status')).toHaveTextContent('Paused: agents are paused');
	});

	it('shows the clarifying questions when answers are needed', () => {
		view(
			loaded(
				makeDetail({
					...makeGoal({ status: 'needs_answers', current_plan_id: null }),
					plan: null,
					questions: [makeQuestion(1), makeQuestion(2)]
				})
			)
		);
		expect(screen.getByLabelText('Question 1?')).toBeInTheDocument();
		expect(screen.getByLabelText('Question 2?')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Submit answers' })).toBeEnabled();
	});

	it('shows a ready plan with Run enabled, its companies and its settings', async () => {
		const props = view(loaded(makeDetail()));
		expect(screen.getByText('Frontend roles at two climate companies')).toBeInTheDocument();
		expect(screen.getByText(/^Version 1 · by the planner · .* · 90% confident · \$0\.01$/)).toBeInTheDocument();
		const companies = screen.getByRole('list', { name: 'Target companies' });
		expect(within(companies).getByText('greenhouse/example-labs')).toBeInTheDocument();
		expect(within(companies).getAllByText('Unverified board')).toHaveLength(1);
		expect(screen.getByText('Lead with design systems')).toBeInTheDocument();
		const steps = screen.getByRole('list', { name: 'Steps' });
		expect(within(steps).getAllByText('Pending')).toHaveLength(5);
		const run = screen.getByRole('button', { name: 'Run' });
		expect(run).toBeEnabled();
		await userEvent.setup().click(run);
		expect(props.onrun).toHaveBeenCalledOnce();
	});

	it('shows an in-review plan with Run disabled and a link to the queue', () => {
		view(loaded(makeDetail({ plan: makePlan({ status: 'in_review', review_item_id: 3 }) })));
		expect(screen.getByText('This plan needs review before it can run.')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Open the review queue' })).toHaveAttribute(
			'href',
			'/review'
		);
		expect(screen.getByRole('button', { name: 'Run' })).toBeDisabled();
	});

	it('shows a dismissed plan', () => {
		view(loaded(makeDetail({ plan: makePlan({ status: 'dismissed' }) })));
		expect(screen.getByText('You dismissed this plan. Edit it or replan.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Run' })).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Replan' })).toBeEnabled();
	});

	it('shows a running plan with each step status and its kept hint', () => {
		const steps = [
			makeStep(0, 'succeeded', {
				output: { companies: 2, unverified: 1 },
				kept_from: 90,
				kept_from_version: 1
			}),
			makeStep(1, 'succeeded', { output: { boards: 2, inserted: 5, updated: 1 } }),
			makeStep(2, 'running'),
			makeStep(3),
			makeStep(4)
		];
		view(
			loaded(
				makeDetail({
					...makeGoal({ status: 'running' }),
					plan: makePlan({ version: 2, trigger: 'user_edit', author: 'user', confidence: null, steps })
				})
			)
		);
		const list = within(screen.getByRole('list', { name: 'Steps' }));
		expect(list.getByText('2 companies, 1 unverified board')).toBeInTheDocument();
		expect(list.getByText('kept from version 1')).toBeInTheDocument();
		expect(list.getByText('2 boards read: 5 new, 1 updated')).toBeInTheDocument();
		expect(list.getByText('Scoring postings…')).toBeInTheDocument();
		expect(list.getAllByText('Done')).toHaveLength(2);
		expect(list.getByText('Running')).toBeInTheDocument();
		expect(screen.getByText(/^Version 2 · edited by you · /)).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Run' })).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Replan' })).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Edit plan' })).toBeEnabled();
	});

	it('shows a completed plan with its shortlist', () => {
		view(
			loaded(
				makeDetail({
					...makeGoal({ status: 'completed' }),
					plan: makePlan({
						steps: [0, 1, 2, 3, 4].map((n) =>
							makeStep(n, n === 4 ? 'skipped' : 'succeeded', {
								output:
									n === 3
										? { shortlisted: 1, in_review: 1 }
										: n === 4
											? { note: "Application prep isn't available yet" }
											: {}
							})
						),
						shortlist: [
							{
								rank: 1,
								posting_id: 42,
								company: 'Example Labs',
								title: 'Senior Frontend Engineer',
								score: 82,
								band: 'strong'
							}
						]
					})
				})
			)
		);
		const shortlist = within(screen.getByRole('list', { name: 'Shortlist' }));
		expect(shortlist.getByRole('link', { name: 'Senior Frontend Engineer' })).toHaveAttribute(
			'href',
			'/postings/42'
		);
		expect(shortlist.getByText('82 · Strong fit')).toBeInTheDocument();
		expect(screen.getByText('1 shortlisted · 1 waiting in review')).toBeInTheDocument();
		expect(screen.getByText("Application prep isn't available yet")).toBeInTheDocument();
	});

	it('shows a failed goal with its error and Run again when the plan is ready', () => {
		view(
			loaded(
				makeDetail({
					...makeGoal({
						status: 'failed',
						error: "source failed: Couldn't source greenhouse/ghost: HTTP 404"
					}),
					plan: makePlan({ trigger: 'step_failed', failed_step: 'source', version: 3 })
				})
			)
		);
		expect(screen.getByRole('alert')).toHaveTextContent('This goal failed');
		expect(screen.getByText(/Couldn't source greenhouse\/ghost/)).toBeInTheDocument();
		expect(screen.getAllByRole('button', { name: 'Run again' })[0]).toBeEnabled();
		expect(screen.getByText(/^Version 3 · by the planner after the source boards step failed/))
			.toBeInTheDocument();
	});

	it('offers Replan for a goal whose planning failed', () => {
		view(
			loaded(
				makeDetail({
					...makeGoal({ status: 'failed', error: 'Planning failed: no valid company is left' }),
					plan: null
				})
			)
		);
		expect(screen.getByText('Planning failed: no valid company is left')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Replan' })).toBeEnabled();
		expect(screen.queryByRole('button', { name: 'Run again' })).not.toBeInTheDocument();
	});
});

describe('GoalStore', () => {
	function setup(...answers: Response[]) {
		const fetch = vi.fn<typeof globalThis.fetch>(async () => answers.shift() ?? json({}, 500));
		const store = new GoalStore(1, { client: createApiClient({ baseUrl: 'http://api.test', fetch }) });
		return { store, fetch };
	}

	const sent = async (fetch: ReturnType<typeof setup>['fetch'], at: number) => {
		const request = fetch.mock.calls[at][0] as Request;
		return {
			method: request.method,
			path: new URL(request.url).pathname,
			body: request.method === 'GET' ? null : await request.clone().json()
		};
	};

	it('loads the goal, and a 404 is not-found', async () => {
		const { store } = setup(json(makeDetail()), json({}, 404));
		await store.load();
		expect(store.state.kind).toBe('loaded');
		await store.load();
		expect(store.state.kind).toBe('not-found');
	});

	it('reloads on goal.updated for its own id only, and after a reconnect or lag', () => {
		const get = vi.fn<() => Promise<Result<GoalDetail>>>(async () => ({
			kind: 'ok',
			data: makeDetail()
		}));
		const store = new GoalStore(1, { get });
		store.handle({ type: 'goal.updated', goal: makeGoal({ id: 2 }) });
		expect(get).not.toHaveBeenCalled();
		store.handle({ type: 'goal.updated', goal: makeGoal({ id: 1 }) });
		store.handle({ type: 'app.connected' });
		store.handle({ type: 'app.lagged', skipped: 3 });
		expect(get).toHaveBeenCalledTimes(3);
	});

	it('runs the plan and shows the answer', async () => {
		const { store, fetch } = setup(json(makeDetail({ ...makeGoal({ status: 'running' }) }), 202));
		const result = await store.run();
		expect(result.kind).toBe('ok');
		expect(await sent(fetch, 0)).toEqual({ method: 'POST', path: '/goals/1/run', body: {} });
		expect(store.state.kind === 'loaded' && store.state.goal.status).toBe('running');
	});

	it('shows the 409 message of Run, Replan and Answers and reloads', async () => {
		const { store, fetch } = setup(
			json({ message: 'This plan needs review before it can run' }, 409),
			json(makeDetail()),
			json({ message: 'Waypoint is already planning this goal' }, 409),
			json(makeDetail()),
			json({ message: "This goal isn't waiting for answers" }, 409),
			json(makeDetail())
		);
		expect(await store.run()).toEqual({
			kind: 'conflict',
			message: 'This plan needs review before it can run'
		});
		await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(2));
		await store.replan('Focus on Series B');
		await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(4));
		await store.answer([{ question_id: 3, answer: 'Backend' }]);
		await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(6));
		expect(await sent(fetch, 2)).toEqual({
			method: 'POST',
			path: '/goals/1/replan',
			body: { instruction: 'Focus on Series B' }
		});
		expect(await sent(fetch, 4)).toEqual({
			method: 'POST',
			path: '/goals/1/answers',
			body: { answers: [{ question_id: 3, answer: 'Backend' }] }
		});
		expect(toasts()).toEqual([
			['danger', 'This plan needs review before it can run'],
			['danger', 'Waypoint is already planning this goal'],
			['danger', "This goal isn't waiting for answers"]
		]);
	});
});
