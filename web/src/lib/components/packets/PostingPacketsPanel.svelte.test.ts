import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type PacketQueued, type PostingPackets } from '#lib/api';
import { makeFitRun, makePacket, makePostingPackets } from '#lib/packets/__fixtures__/packets.js';
import { packetsView, type Result } from '#lib/packets/api.js';
import { PostingPacketsStore, type PostingPacketsState } from '#lib/packets/packets.svelte.js';
import PostingPacketsPanel from './PostingPacketsPanel.svelte';

function loaded(packets: Partial<PostingPackets>): PostingPacketsState {
	const full = makePostingPackets(packets);
	return { kind: 'loaded', packets: full, view: packetsView(full) };
}

function setup(state: PostingPacketsState, starting = false) {
	const onprepare = vi.fn();
	const onreload = vi.fn();
	const result = render(PostingPacketsPanel, {
		state,
		starting,
		onprepare,
		onreload
	});
	return { ...result, onprepare, onreload };
}

const button = (name: string) => screen.getByRole('button', { name });

describe('PostingPacketsPanel', () => {
	it('shows a skeleton while loading', () => {
		setup({ kind: 'loading' });
		expect(screen.getByLabelText('Loading packets')).toBeInTheDocument();
		expect(screen.queryByRole('button')).toBeNull();
	});

	it('shows a load error with Try again', async () => {
		const { onreload, onprepare } = setup({ kind: 'error' });
		expect(screen.getByText("Couldn't load packets")).toBeInTheDocument();
		await userEvent.click(button('Try again'));
		expect(onreload).toHaveBeenCalledTimes(1);
		expect(onprepare).not.toHaveBeenCalled();
	});

	it('disables Prepare packet until the posting is scored', () => {
		setup(loaded({ scored: false }));
		expect(
			screen.getByText("Score this posting's fit before preparing a packet.")
		).toBeInTheDocument();
		expect(button('Prepare packet')).toBeDisabled();
	});

	it('shows the empty state with Prepare packet', async () => {
		const { onprepare } = setup(loaded({}));
		expect(screen.getByText('No packet yet')).toBeInTheDocument();
		await userEvent.click(button('Prepare packet'));
		expect(onprepare).toHaveBeenCalledTimes(1);
	});

	it.each([
		['queued', 'Preparing…'],
		['running', 'Preparing…'],
		['paused', 'Paused: agents are paused']
	] as const)('shows a %s run as preparing, with the button disabled', (status, text) => {
		setup(loaded({ run: makeFitRun(status, 60) }));
		expect(screen.getByRole('status')).toHaveTextContent(text);
		expect(button('Preparing…')).toBeDisabled();
		expect(screen.getByRole('link', { name: 'Run #60' })).toHaveAttribute('href', '/runs/60');
	});

	it('shows a failed run with its error, its run and Try again', async () => {
		const { onprepare } = setup(loaded({ run: makeFitRun('failed', 60, 'step "prepare": boom') }));
		expect(screen.getByRole('alert')).toHaveTextContent('Preparing failed');
		expect(screen.getByText('step "prepare": boom')).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Run #60' })).toHaveAttribute('href', '/runs/60');
		await userEvent.click(button('Try again'));
		expect(onprepare).toHaveBeenCalledTimes(1);
	});

	it('lists packets newest first with status, resume, answered count and a link', async () => {
		const missing = makePacket({
			id: 4,
			run_id: 51,
			status: 'in_review',
			resume_name: 'Frontend'
		});
		missing.answers[2] = { ...missing.answers[2], answer: null, citations: [] };
		const { onprepare } = setup(
			loaded({
				packets: [missing, makePacket({ status: 'dismissed' })],
				run: makeFitRun('succeeded', 51)
			})
		);
		const rows = within(screen.getByRole('list', { name: 'Packets' })).getAllByRole('listitem');
		expect(rows).toHaveLength(2);
		expect(within(rows[0]).getByText('Needs review')).toBeInTheDocument();
		expect(within(rows[0]).getByText('Frontend')).toBeInTheDocument();
		expect(within(rows[0]).getByText('4 of 5 answered')).toBeInTheDocument();
		expect(within(rows[0]).getByText(/^Prepared /)).toBeInTheDocument();
		expect(within(rows[0]).getByRole('link', { name: 'View packet' })).toHaveAttribute(
			'href',
			'/packets/4'
		);
		expect(within(rows[1]).getByText('Dismissed')).toBeInTheDocument();
		expect(within(rows[1]).getByText('5 of 5 answered')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Prepare packet' })).toBeNull();
		await userEvent.click(button('Prepare again'));
		expect(onprepare).toHaveBeenCalledTimes(1);
	});

	it('shows a ready packet as Ready for approval', () => {
		setup(loaded({ packets: [makePacket()] }));
		expect(screen.getByText('Ready for approval')).toBeInTheDocument();
	});

	it('shows the preparing state above the list while a run prepares', () => {
		setup(loaded({ packets: [makePacket()], run: makeFitRun('running', 60) }));
		expect(screen.getByRole('status')).toHaveTextContent('Preparing…');
		expect(screen.getByText('Ready for approval')).toBeInTheDocument();
		expect(button('Prepare again')).toBeDisabled();
	});

	it('disables Prepare again while a request is in flight', () => {
		setup(loaded({ packets: [makePacket()] }), true);
		expect(button('Prepare again')).toBeDisabled();
	});

	it('posts on Prepare packet and switches to the preparing state', async () => {
		const client = createApiClient({
			baseUrl: 'http://api.test',
			fetch: vi.fn()
		});
		const get = vi.fn(async (): Promise<Result<PostingPackets>> => ({
			kind: 'ok',
			data: makePostingPackets()
		}));
		const prepare = vi.fn(async (): Promise<Result<PacketQueued>> => ({
			kind: 'ok',
			data: {
				reused: false,
				run: {
					id: 61,
					workflow: 'prep_packet',
					status: 'queued',
					input: { posting_id: 9 },
					output: null,
					error: null,
					input_tokens: 0,
					output_tokens: 0,
					cost_usd: 0,
					job_id: 1,
					created_at: '2026-10-22T00:00:00Z',
					updated_at: '2026-10-22T00:00:00Z',
					started_at: null,
					finished_at: null
				}
			}
		}));
		const store = new PostingPacketsStore(9, { client, get, prepare });
		await store.load();
		const { rerender } = render(PostingPacketsPanel, {
			state: store.state,
			starting: store.starting,
			onprepare: () => store.prepare(),
			onreload: vi.fn()
		});
		await userEvent.click(button('Prepare packet'));
		expect(prepare).toHaveBeenCalledWith(client, 9);
		await rerender({ state: store.state, starting: store.starting });
		expect(screen.getByRole('status')).toHaveTextContent('Preparing…');
		expect(button('Preparing…')).toBeDisabled();
	});
});
