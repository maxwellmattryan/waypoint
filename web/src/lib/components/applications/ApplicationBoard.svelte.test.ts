import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Application, ApplicationStatus } from '#lib/api';
import { STATUSES } from '#lib/applications/statuses.js';
import ApplicationBoard from './ApplicationBoard.svelte';

function row(id: number, status: ApplicationStatus, overrides: Partial<Application> = {}): Application {
	return {
		id,
		company: `Company ${id}`,
		title: 'Engineer',
		url: null,
		location: null,
		comp: null,
		source: null,
		notes: null,
		status,
		status_changed_at: '2026-10-22T00:00:00Z',
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		...overrides
	};
}

function columnsOf(applications: Application[]) {
	return STATUSES.map((status) => ({
		status,
		applications: applications.filter((a) => a.status === status)
	}));
}

const rows = [
	row(1, 'sourced', { location: 'Remote' }),
	row(2, 'sourced'),
	row(3, 'applied'),
	row(4, 'offer')
];

function setup(pending: number[] = []) {
	const onmove = vi.fn();
	render(ApplicationBoard, { columns: columnsOf(rows), pending: new Set(pending), onmove });
	return { onmove };
}

function column(name: string) {
	return screen.getByRole('region', { name });
}

function card(id: number) {
	return screen.getByRole('link', { name: `Company ${id} · Engineer` }).closest('[aria-busy]')!;
}

function zone(name: string) {
	return within(column(name)).getByRole('list').parentElement!;
}

describe('ApplicationBoard', () => {
	it('renders five labelled columns in board order with counts and placeholders', () => {
		setup();
		const regions = screen.getAllByRole('region');
		expect(regions.map((r) => within(r).getByRole('heading', { level: 2 }).textContent)).toEqual([
			'Sourced',
			'Applied',
			'Interviewing',
			'Offer',
			'Rejected'
		]);
		const counts = { Sourced: 2, Applied: 1, Interviewing: 0, Offer: 1, Rejected: 0 };
		for (const [name, count] of Object.entries(counts)) {
			const region = column(name);
			expect(within(region).getAllByText(String(count))[0]).toBeInTheDocument();
			expect(within(region).queryAllByRole('listitem')).toHaveLength(count);
			const placeholder = within(region).queryByText('No applications');
			if (count === 0) expect(placeholder).toBeInTheDocument();
			else expect(placeholder).toBeNull();
		}

		const first = within(column('Sourced')).getAllByRole('listitem')[0];
		expect(within(first).getByRole('link', { name: 'Company 1 · Engineer' })).toHaveAttribute(
			'href',
			'/applications/1'
		);
		expect(first).toHaveTextContent('Remote');
		expect(first).toHaveTextContent(/Updated /);
		expect(
			within(first).getByRole('button', { name: 'Move Company 1 · Engineer' })
		).toBeInTheDocument();
		expect(card(1)).toHaveAttribute('draggable', 'true');
		expect(card(1)).toHaveAttribute('aria-busy', 'false');
	});

	it('dropping on another column moves the card; its own column does not', async () => {
		const { onmove } = setup();
		await fireEvent.dragStart(card(1));
		await fireEvent.dragOver(zone('Sourced'));
		expect(zone('Sourced')).not.toHaveAttribute('data-over');
		await fireEvent.drop(zone('Sourced'));
		expect(onmove).not.toHaveBeenCalled();

		await fireEvent.dragStart(card(1));
		await fireEvent.dragOver(zone('Interviewing'));
		expect(zone('Interviewing')).toHaveAttribute('data-over', 'true');
		expect(zone('Interviewing').className).toContain('bg-primary-soft');
		await fireEvent.dragLeave(zone('Interviewing'));
		expect(zone('Interviewing')).not.toHaveAttribute('data-over');
		await fireEvent.dragOver(zone('Interviewing'));
		await fireEvent.drop(zone('Interviewing'));
		expect(onmove).toHaveBeenCalledExactlyOnceWith(1, 'interviewing');
		expect(zone('Interviewing')).not.toHaveAttribute('data-over');

		// A drop with no drag in progress (e.g. a file from outside) does nothing.
		await fireEvent.drop(zone('Rejected'));
		expect(onmove).toHaveBeenCalledOnce();
	});

	it("picking a status from a card's Move menu moves it", async () => {
		const user = userEvent.setup();
		const { onmove } = setup();
		await user.click(screen.getByRole('button', { name: 'Move Company 3 · Engineer' }));
		const menu = screen.getByRole('menu', { name: 'Move to' });
		expect(within(menu).getAllByRole('menuitem').map((el) => el.textContent?.trim())).toEqual([
			'Sourced',
			'Interviewing',
			'Offer',
			'Rejected'
		]);
		await user.click(within(menu).getByRole('menuitem', { name: 'Offer' }));
		expect(onmove).toHaveBeenCalledExactlyOnceWith(3, 'offer');
		await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
	});

	it('a pending card is busy, not draggable and its Move button is disabled', () => {
		setup([2]);
		expect(card(2)).toHaveAttribute('aria-busy', 'true');
		expect(card(2)).not.toHaveAttribute('draggable');
		expect(screen.getByRole('button', { name: 'Move Company 2 · Engineer' })).toBeDisabled();
		expect(card(1)).toHaveAttribute('draggable', 'true');
		expect(screen.getByRole('button', { name: 'Move Company 1 · Engineer' })).toBeEnabled();
	});
});
