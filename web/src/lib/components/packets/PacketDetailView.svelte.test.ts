import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { ApplicationPacket } from '#lib/api';
import { memoryHref } from '#lib/memories/link.js';
import { makePacket } from '#lib/packets/__fixtures__/packets.js';
import PacketDetailView from './PacketDetailView.svelte';

function setup(overrides: Partial<ApplicationPacket> = {}) {
	return render(PacketDetailView, { packet: makePacket(overrides) });
}

const answer = (container: HTMLElement, key: string) => {
	const item = container.querySelector<HTMLElement>(`[data-answer="${key}"]`);
	if (!item) throw new Error(`no answer ${key}`);
	return within(item);
};

describe('PacketDetailView', () => {
	it('shows the header with the posting, status, confidence, cost and run', () => {
		setup();
		expect(
			screen.getByRole('heading', { level: 1, name: 'Application packet' })
		).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Acme — Backend Engineer' })).toHaveAttribute(
			'href',
			'/postings/9'
		);
		expect(screen.getByText('Ready for approval')).toBeInTheDocument();
		expect(screen.getByText('90% confident')).toBeInTheDocument();
		expect(screen.getByText(/Prepared .* · \$0\.01/)).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Run #50' })).toHaveAttribute('href', '/runs/50');
		expect(screen.queryByText('This packet is waiting in review')).toBeNull();
	});

	it('says when the packet waits in review', () => {
		setup({ status: 'in_review', review_item_id: 4 });
		expect(screen.getByText('Needs review')).toBeInTheDocument();
		expect(screen.getByText('This packet is waiting in review')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Open the review queue' })).toHaveAttribute(
			'href',
			'/review'
		);
	});

	it('shows the fit snapshot, or Not scored', () => {
		const { unmount } = setup();
		expect(screen.getByText('82 · Strong fit')).toBeInTheDocument();
		unmount();
		setup({ fit_score_id: null, fit_score: null, fit_band: null });
		expect(screen.getByText('Not scored')).toBeInTheDocument();
	});

	it('links the chosen resume and shows its reasons with citations', () => {
		setup();
		expect(screen.getByRole('link', { name: 'Backend' })).toHaveAttribute('href', '/resumes/1');
		const reasons = within(screen.getByRole('list', { name: 'Why this resume' }));
		expect(reasons.getByText('Matches the Rust role')).toBeInTheDocument();
		expect(reasons.getByText('“Rust”')).toBeInTheDocument();
		expect(reasons.getByRole('link', { name: 'View memory' })).toHaveAttribute(
			'href',
			memoryHref(5)
		);
	});

	it('handles a deleted resume and a deleted memory', () => {
		const packet = makePacket({ resume_version_id: null });
		packet.resume_reasons[0].citations[1] = {
			...packet.resume_reasons[0].citations[1],
			memory_id: null
		};
		render(PacketDetailView, { packet });
		expect(screen.queryByRole('link', { name: 'Backend' })).toBeNull();
		expect(screen.getByText('Backend')).toBeInTheDocument();
		expect(screen.getByText('Deleted resume')).toBeInTheDocument();
		expect(screen.getByText('Deleted memory')).toBeInTheDocument();
		expect(screen.getByText('Shipped Rust services')).toBeInTheDocument();
	});

	it('shows each answer with its citations, and marks a missing one', () => {
		const packet = makePacket();
		packet.answers[2] = { ...packet.answers[2], answer: null, citations: [] };
		const { container } = render(PacketDetailView, { packet });
		const work = answer(container, 'work_authorization');
		expect(
			work.getByText(
				"Are you authorized to work in the role's location, and will you need visa sponsorship?"
			)
		).toBeInTheDocument();
		expect(work.getByText('Yes. I am an EU citizen.')).toBeInTheDocument();
		expect(work.getByRole('link', { name: 'View memory' })).toHaveAttribute('href', memoryHref(1));
		const salary = answer(container, 'salary_expectation');
		expect(salary.getByText('Missing')).toBeInTheDocument();
		expect(salary.getByText('Waypoint had nothing to draft this from.')).toBeInTheDocument();
		expect(salary.queryByRole('list', { name: 'Evidence' })).toBeNull();
		expect(answer(container, 'why_company').getByText('“Rust”')).toBeInTheDocument();
	});

	it('renders answers and excerpts as literal text', () => {
		const packet = makePacket();
		packet.answers[0] = {
			...packet.answers[0],
			answer: '<b>Yes</b>\nLine two',
			citations: [
				{
					source: 'posting',
					memory_id: null,
					topic: null,
					text: '<i>Rust</i>'
				}
			]
		};
		const { container } = render(PacketDetailView, { packet });
		expect(container.querySelector('[data-answer] b')).toBeNull();
		expect(container.querySelector('blockquote i')).toBeNull();
		expect(screen.getByText(/<b>Yes<\/b>/)).toBeInTheDocument();
		expect(screen.getByText('“<i>Rust</i>”')).toBeInTheDocument();
	});

	it('has no edit, approve or submit control', () => {
		setup({ status: 'in_review' });
		expect(screen.queryByRole('button')).toBeNull();
		expect(screen.queryByRole('textbox')).toBeNull();
	});
});
