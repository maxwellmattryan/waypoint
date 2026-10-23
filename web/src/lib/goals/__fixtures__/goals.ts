// Goal and plan bodies for the goals tests, shaped like `GET /goals/{id}`.
import type {
	Goal,
	GoalDetail,
	GoalQuestion,
	Plan,
	PlanPacket,
	PlanStep,
	PlanStepKind,
	PlanStepStatus
} from '#lib/api';

const KINDS: PlanStepKind[] = ['target_companies', 'source', 'score', 'shortlist', 'prep'];

export function makeStep(
	ordinal: number,
	status: PlanStepStatus = 'pending',
	overrides: Partial<PlanStep> = {}
): PlanStep {
	return {
		id: 100 + ordinal,
		ordinal,
		kind: KINDS[ordinal],
		status,
		config: {},
		output: null,
		last_error: null,
		kept_from: null,
		kept_from_version: null,
		started_at: null,
		finished_at: null,
		...overrides
	};
}

export function makePlan(overrides: Partial<Plan> = {}): Plan {
	return {
		id: 11,
		goal_id: 1,
		version: 1,
		author: 'planner',
		trigger: 'goal',
		status: 'ready',
		summary: 'Frontend roles at two climate companies',
		confidence: 0.9,
		companies: [
			{
				name: 'Example Labs',
				source: 'greenhouse',
				board: 'example-labs',
				reason: 'Builds grid software',
				verified: true
			},
			{ name: 'Watershed', source: 'ashby', board: 'watershed', reason: null, verified: false }
		],
		title_keywords: ['frontend'],
		locations: ['new york'],
		remote_only: false,
		max_to_score: 20,
		shortlist_size: 5,
		min_score: 60,
		prep_notes: 'Lead with design systems',
		dropped: { companies: [], keywords: [], locations: [] },
		run_id: 4,
		review_item_id: null,
		claude_call_id: 9,
		cost_usd: 0.0123,
		failed_step: null,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		steps: [0, 1, 2, 3, 4].map((ordinal) => makeStep(ordinal)),
		shortlist: [],
		packets: [],
		...overrides
	};
}

export function makePlanPacket(overrides: Partial<PlanPacket> = {}): PlanPacket {
	return {
		posting_id: 42,
		company: 'Example Labs',
		title: 'Senior Frontend Engineer',
		outcome: 'prepared',
		packet_id: 7,
		status: 'ready',
		run_id: 30,
		error: null,
		...overrides
	};
}

export function makeQuestion(id: number, overrides: Partial<GoalQuestion> = {}): GoalQuestion {
	return {
		id,
		ordinal: id,
		question: `Question ${id}?`,
		answer: null,
		answered_at: null,
		...overrides
	};
}

export function makeGoal(overrides: Partial<Goal> = {}): Goal {
	return {
		id: 1,
		text: 'Senior frontend roles at climate companies in NYC',
		status: 'planned',
		error: null,
		auto_replans: 0,
		current_plan_id: 11,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		...overrides
	};
}

export function makeDetail(overrides: Partial<GoalDetail> = {}): GoalDetail {
	return {
		...makeGoal(),
		questions: [],
		plan: makePlan(),
		versions: [],
		run: { id: 4, status: 'succeeded', error: null, created_at: '2026-10-22T00:00:00Z' },
		...overrides
	};
}
