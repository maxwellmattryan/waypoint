import type { ApplicationPacket, FitRun, PacketAnswer, PostingPackets } from '#lib/api';

/** Test data: a packet's five answers, all drafted and cited. */
export function makeAnswers(): PacketAnswer[] {
	return [
		{
			key: 'work_authorization',
			question:
				"Are you authorized to work in the role's location, and will you need visa sponsorship?",
			answer: 'Yes. I am an EU citizen.',
			citations: [
				{
					source: 'memory',
					memory_id: 1,
					topic: 'work_authorization',
					text: 'EU citizen'
				}
			]
		},
		{
			key: 'notice_period',
			question: 'When could you start?',
			answer: 'Four weeks after an offer.',
			citations: [
				{
					source: 'memory',
					memory_id: 2,
					topic: 'notice_period',
					text: 'Four weeks'
				}
			]
		},
		{
			key: 'salary_expectation',
			question: 'What are your salary expectations?',
			answer: 'Around 90,000 EUR.',
			citations: [
				{
					source: 'memory',
					memory_id: 3,
					topic: 'salary_expectation',
					text: '90,000 EUR'
				}
			]
		},
		{
			key: 'why_leaving',
			question: 'Why are you looking to leave your current role?',
			answer: 'I want more scope.',
			citations: [
				{
					source: 'memory',
					memory_id: 4,
					topic: 'why_leaving',
					text: 'More scope'
				}
			]
		},
		{
			key: 'why_company',
			question: 'Why do you want to work at Acme?',
			answer: 'You build Rust services.',
			citations: [{ source: 'posting', memory_id: null, topic: null, text: 'Rust' }]
		}
	];
}

/** Test data: a ready packet for posting 9 at Acme. */
export function makePacket(overrides: Partial<ApplicationPacket> = {}): ApplicationPacket {
	return {
		id: 3,
		posting_id: 9,
		company: 'Acme',
		title: 'Backend Engineer',
		run_id: 50,
		status: 'ready',
		resume_version_id: 1,
		resume_name: 'Backend',
		resume_reasons: [
			{
				text: 'Matches the Rust role',
				citations: [
					{ source: 'posting', memory_id: null, topic: null, text: 'Rust' },
					{
						source: 'memory',
						memory_id: 5,
						topic: 'skill',
						text: 'Shipped Rust services'
					}
				]
			}
		],
		answers: makeAnswers(),
		fit_score_id: 7,
		fit_score: 82,
		fit_band: 'strong',
		confidence: 0.9,
		review_item_id: null,
		cost_usd: 0.0123,
		created_at: '2026-10-22T10:00:00Z',
		updated_at: '2026-10-22T10:00:00Z',
		approved_resume_version_id: null,
		approved_resume_name: null,
		approved_answers: null,
		approved_at: null,
		application_id: null,
		applied_at: null,
		pending_action_id: null,
		undo_action_id: null,
		latest_run: null,
		...overrides
	};
}

/** Test data: packet 3 approved with resume 2 "Frontend" and notice_period edited. */
export function makeApprovedPacket(overrides: Partial<ApplicationPacket> = {}): ApplicationPacket {
	const approved = makeAnswers();
	approved[1] = { ...approved[1], answer: 'In two weeks.', citations: [] };
	return makePacket({
		status: 'approved',
		approved_resume_version_id: 2,
		approved_resume_name: 'Frontend',
		approved_answers: approved,
		approved_at: '2026-10-22T11:00:00Z',
		undo_action_id: 70,
		latest_run: makeFitRun('succeeded', 60),
		...overrides
	});
}

export const makeFitRun = (
	status: FitRun['status'],
	id = 50,
	error: string | null = null
): FitRun => ({ id, status, error });

/** Test data: posting 9's packets body. */
export function makePostingPackets(overrides: Partial<PostingPackets> = {}): PostingPackets {
	return { posting_id: 9, scored: true, packets: [], run: null, ...overrides };
}
