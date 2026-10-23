import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { PlanPacket } from '#lib/api';
import { makePlanPacket } from '#lib/goals/__fixtures__/goals.js';
import PlanPacketList from './PlanPacketList.svelte';

function rows(packets: PlanPacket[]) {
	render(PlanPacketList, { packets });
	return within(screen.getByRole('list', { name: 'Packets' })).getAllByRole('listitem');
}

const INVALID = 'the packet is invalid: resume_id must be one of the resumes shown';

describe('PlanPacketList', () => {
	it('links each posting with its company', () => {
		const [row] = rows([makePlanPacket()]);
		expect(within(row).getByRole('link', { name: 'Senior Frontend Engineer' })).toHaveAttribute(
			'href',
			'/postings/42'
		);
		expect(within(row).getByText('Example Labs')).toBeInTheDocument();
	});

	it('shows a ready packet with Review and approve, and an approved one with View packet', () => {
		const [ready, approved] = rows([
			makePlanPacket(),
			makePlanPacket({ posting_id: 43, packet_id: 8, status: 'approved', outcome: 'reused' })
		]);
		expect(within(ready).getByText('Ready for approval')).toBeInTheDocument();
		expect(within(ready).getByRole('link', { name: 'Review and approve' })).toHaveAttribute(
			'href',
			'/packets/7'
		);
		expect(within(approved).getByText('Approved')).toBeInTheDocument();
		expect(within(approved).getByRole('link', { name: 'View packet' })).toHaveAttribute(
			'href',
			'/packets/8'
		);
		expect(within(approved).queryByRole('link', { name: 'Review and approve' })).toBeNull();
	});

	it('shows a queued entry with its run and a failed one with its error', () => {
		const [queued, failed] = rows([
			makePlanPacket({ outcome: 'queued', packet_id: null, status: null, run_id: 31 }),
			makePlanPacket({
				posting_id: 43,
				outcome: 'failed',
				packet_id: null,
				status: null,
				run_id: 32,
				error: INVALID
			})
		]);
		expect(within(queued).getByText('Preparing…')).toBeInTheDocument();
		expect(within(queued).getByRole('link', { name: 'Run #31' })).toHaveAttribute('href', '/runs/31');
		expect(within(failed).getByText("Couldn't prepare")).toBeInTheDocument();
		expect(within(failed).getByText(INVALID)).toBeInTheDocument();
		expect(within(failed).getByRole('link', { name: 'Run #32' })).toHaveAttribute('href', '/runs/32');
	});

	it('shows a failed entry without a run, and a removed packet without a link', () => {
		const [failed, removed] = rows([
			makePlanPacket({
				outcome: 'failed',
				packet_id: null,
				status: null,
				run_id: null,
				error: 'The posting has no applied fit score'
			}),
			makePlanPacket({ posting_id: 43, status: null })
		]);
		expect(within(failed).getByText('The posting has no applied fit score')).toBeInTheDocument();
		expect(within(failed).queryByRole('link', { name: /^Run #/ })).toBeNull();
		expect(within(removed).getByText('Packet removed')).toBeInTheDocument();
		expect(within(removed).getAllByRole('link')).toHaveLength(1);
	});
});
