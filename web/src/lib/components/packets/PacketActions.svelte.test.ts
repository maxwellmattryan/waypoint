import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ApplicationPacket } from '#lib/api';
import { makeApprovedPacket, makeFitRun, makePacket } from '#lib/packets/__fixtures__/packets.js';
import type { PacketAction } from '#lib/packets/packets.svelte.js';
import PacketActions from './PacketActions.svelte';

function setup(packet: ApplicationPacket, busy: PacketAction | null = null) {
	const props = {
		packet,
		busy,
		onapprove: vi.fn(),
		onedit: vi.fn(),
		onreject: vi.fn(async () => {}),
		onrequest: vi.fn(),
		onapply: vi.fn(),
		onundo: vi.fn()
	};
	render(PacketActions, props);
	return props;
}

const buttons = () => screen.queryAllByRole('button').map((button) => button.textContent?.trim());
const button = (name: string) => screen.getByRole('button', { name });

describe('PacketActions', () => {
	it('asks to clear review first for an in_review packet, without buttons', () => {
		setup(makePacket({ status: 'in_review' }));
		expect(
			screen.getByText('Accept or dismiss its review item before approving.')
		).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Go to Review' })).toHaveAttribute('href', '/review');
		expect(buttons()).toEqual([]);
	});

	it('approves, edits and rejects (through its confirm dialog) a pending approval', async () => {
		const props = setup(makePacket({ pending_action_id: 80 }));
		expect(buttons()).toEqual(['Approve', 'Edit', 'Reject']);
		await userEvent.click(button('Approve'));
		expect(props.onapprove).toHaveBeenCalledTimes(1);
		await userEvent.click(button('Edit'));
		expect(props.onedit).toHaveBeenCalledTimes(1);

		await userEvent.click(button('Reject'));
		expect(props.onreject).not.toHaveBeenCalled();
		const dialog = screen.getByRole('dialog', { name: 'Reject this packet?' });
		expect(dialog).toHaveAccessibleDescription(
			'It will be dismissed, and Waypoint learns from the rejection.'
		);
		const confirm = screen
			.getAllByRole('button', { name: 'Reject' })
			.find((element) => dialog.contains(element));
		if (!confirm) throw new Error('no confirm button');
		await userEvent.click(confirm);
		expect(props.onreject).toHaveBeenCalledTimes(1);
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
	});

	it('disables every button while an action is in flight, with its pending label', () => {
		setup(makePacket({ pending_action_id: 80 }), 'approve');
		expect(button('Approving…')).toBeDisabled();
		expect(button('Edit')).toBeDisabled();
		expect(button('Reject')).toBeDisabled();
	});

	it.each([
		['queued', 'Getting the approval ready…'],
		['running', 'Getting the approval ready…'],
		['paused', 'Paused: agents are paused']
	] as const)('waits while the approval run is %s', (status, text) => {
		setup(makePacket({ latest_run: makeFitRun(status, 61) }));
		expect(screen.getByText(text)).toBeInTheDocument();
		expect(buttons()).toEqual([]);
	});

	it('requests approval, and shows the failed request', async () => {
		const props = setup(makePacket({ latest_run: makeFitRun('failed', 61, 'the run broke') }));
		expect(screen.getByText("Couldn't request approval")).toBeInTheDocument();
		expect(screen.getByText('the run broke')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Run #61' })).toHaveAttribute('href', '/runs/61');
		await userEvent.click(button('Request approval'));
		expect(props.onrequest).toHaveBeenCalledTimes(1);
	});

	it('marks an approved packet applied or undoes its approval', async () => {
		const props = setup(makeApprovedPacket());
		expect(
			screen.getByText(
				"Waypoint doesn't submit anything. Apply on the company's site, then mark it applied."
			)
		).toBeInTheDocument();
		expect(buttons()).toEqual(['Mark applied', 'Undo approval']);
		await userEvent.click(button('Mark applied'));
		expect(props.onapply).toHaveBeenCalledTimes(1);
		await userEvent.click(button('Undo approval'));
		expect(props.onundo).toHaveBeenCalledTimes(1);
	});

	it('blocks mark applied when the approved resume was deleted', () => {
		setup(makeApprovedPacket({ approved_resume_version_id: null }));
		expect(
			screen.getByText('The approved resume was deleted. Undo the approval to choose another.')
		).toBeInTheDocument();
		expect(button('Mark applied')).toBeDisabled();
		expect(button('Undo approval')).toBeEnabled();
	});

	it('waits while marking applied, and offers Try again after a failure', () => {
		const { unmount } = render(PacketActions, {
			packet: makeApprovedPacket({ latest_run: makeFitRun('running', 62) }),
			onapprove: vi.fn(),
			onedit: vi.fn(),
			onreject: vi.fn(),
			onrequest: vi.fn(),
			onapply: vi.fn(),
			onundo: vi.fn()
		});
		expect(screen.getByText('Marking applied…')).toBeInTheDocument();
		expect(button('Mark applied')).toBeDisabled();
		unmount();
		setup(
			makeApprovedPacket({
				latest_run: makeFitRun('failed', 62, 'apply broke')
			})
		);
		expect(screen.getByText('apply broke')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Run #62' })).toBeInTheDocument();
		expect(button('Try again')).toBeEnabled();
	});

	it('shows an applied packet with its application and Undo', async () => {
		const props = setup(
			makeApprovedPacket({
				status: 'applied',
				application_id: 12,
				applied_at: '2026-10-23T09:00:00Z',
				undo_action_id: 71
			})
		);
		expect(screen.getByText(/^Applied /)).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'View application' })).toHaveAttribute(
			'href',
			'/applications/12'
		);
		await userEvent.click(button('Undo'));
		expect(props.onundo).toHaveBeenCalledTimes(1);
	});

	it('says when the applied application was deleted', () => {
		setup(
			makeApprovedPacket({
				status: 'applied',
				application_id: null,
				applied_at: '2026-10-23T09:00:00Z'
			})
		);
		expect(screen.getByText('Application deleted')).toBeInTheDocument();
		expect(screen.queryByRole('link', { name: 'View application' })).toBeNull();
	});

	it('shows a dismissed packet without buttons', () => {
		setup(makePacket({ status: 'dismissed' }));
		expect(screen.getByText('This packet was dismissed.')).toBeInTheDocument();
		expect(buttons()).toEqual([]);
	});
});
