import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { GoogleStatus, Job } from '#lib/api';
import type { TestState } from '#lib/dashboard/dashboard.svelte.js';
import { toast } from '#lib/components/ui';
import AgentsCard from './AgentsCard.svelte';
import ClaudeCard from './ClaudeCard.svelte';
import DataCard from './DataCard.svelte';
import DeleteAllDataDialog from './DeleteAllDataDialog.svelte';
import GoogleCard from './GoogleCard.svelte';
import SystemCard from './SystemCard.svelte';

afterEach(() => {
	vi.restoreAllMocks();
	toast.clear();
});

function job(overrides: Partial<Job> = {}): Job {
	return {
		id: 7,
		kind: 'test_prompt',
		status: 'running',
		attempts: 1,
		max_attempts: 3,
		run_at: '2026-10-22T00:00:00Z',
		last_error: null,
		result: null,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		finished_at: null,
		...overrides
	};
}

function google(overrides: Partial<GoogleStatus>): GoogleStatus {
	return { configured: true, state: 'disconnected', connected_at: null, scopes: [], ...overrides };
}

/** The value badge next to a System card row label. */
function rowValue(label: string): string | null {
	return screen.getByText(label).nextElementSibling?.textContent?.trim() ?? null;
}

describe('SystemCard', () => {
	it('shows skeletons, then ok, degraded and unreachable rows', async () => {
		const { rerender } = render(SystemCard, { health: { kind: 'loading' } });
		expect(screen.getByTestId('system-loading')).toBeInTheDocument();

		await rerender({ health: { kind: 'ok' } });
		expect([rowValue('API'), rowValue('Database')]).toEqual(['OK', 'OK']);

		await rerender({ health: { kind: 'degraded' } });
		expect([rowValue('API'), rowValue('Database')]).toEqual(['OK', 'Unavailable']);

		await rerender({ health: { kind: 'unreachable' } });
		expect([rowValue('API'), rowValue('Database')]).toEqual(['Unreachable', 'Unknown']);
	});
});

describe('GoogleCard', () => {
	it('GoogleCard states', async () => {
		const user = userEvent.setup();
		const assign = vi.fn();
		vi.spyOn(window, 'location', 'get').mockReturnValue({ ...window.location, assign });

		const { rerender } = render(GoogleCard, {
			google: { kind: 'loaded', status: google({ configured: false }) }
		});
		const status = () => screen.getByTestId('google-status');
		expect(status()).toHaveTextContent('Not configured');
		expect(screen.getByText('WAYPOINT_GOOGLE_CLIENT_ID')).toBeInTheDocument();
		expect(screen.getByText('docs/google-oauth.md')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Connect Google' })).toBeDisabled();

		await rerender({ google: { kind: 'loaded', status: google({}) } });
		expect(status()).toHaveTextContent('Not connected');
		const connect = screen.getByRole('button', { name: 'Connect Google' });
		expect(connect).toBeEnabled();
		await user.click(connect);
		expect(assign).toHaveBeenCalledWith('/api/auth/google/connect');

		const connectedAt = '2026-10-20T12:00:00Z';
		await rerender({
			google: {
				kind: 'loaded',
				status: google({
					state: 'connected',
					connected_at: connectedAt,
					scopes: [
						'https://www.googleapis.com/auth/calendar.events',
						'https://www.googleapis.com/auth/gmail.readonly',
						'openid'
					]
				})
			}
		});
		expect(status()).toHaveTextContent('Connected');
		expect(screen.getByText(new Date(connectedAt).toLocaleString())).toBeInTheDocument();
		expect(screen.getByText('Gmail (read-only)')).toBeInTheDocument();
		expect(screen.getByText('Calendar events')).toBeInTheDocument();
		expect(screen.getByText('openid')).toBeInTheDocument();
		expect(screen.queryByRole('button')).toBeNull();

		await rerender({ google: { kind: 'loaded', status: google({ state: 'reconnect_required' }) } });
		expect(status()).toHaveTextContent('Reconnect required');
		await user.click(screen.getByRole('button', { name: 'Reconnect Google' }));
		expect(assign).toHaveBeenCalledTimes(2);

		await rerender({ google: { kind: 'error' } });
		expect(status()).toHaveTextContent('Unavailable');
		expect(screen.getByText("Couldn't load the Google status.")).toBeInTheDocument();

		await rerender({ google: { kind: 'loading' } });
		expect(screen.getByTestId('google-loading')).toBeInTheDocument();
	});
});

function renderClaude(test: TestState, testing = false, queuePausedUntil: string | null = null) {
	const ontest = vi.fn();
	const result = render(ClaudeCard, { test, testing, queuePausedUntil, ontest });
	return { ...result, ontest };
}

describe('ClaudeCard', () => {
	it('ClaudeCard disables while in flight', async () => {
		const user = userEvent.setup();
		const { rerender, ontest } = renderClaude({ kind: 'idle' });
		expect(
			screen.getByText((_, el) => el?.tagName === 'P' && /through claude -p on your/.test(el.textContent ?? ''))
		).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Test Claude' }));
		expect(ontest).toHaveBeenCalledOnce();

		await rerender({ test: { kind: 'starting' }, testing: true });
		expect(screen.getByRole('button', { name: 'Testing…' })).toBeDisabled();

		await rerender({
			test: { kind: 'tracking', jobId: 7, job: null, phase: 'queued' },
			testing: true
		});
		expect(screen.getByTestId('claude-status')).toHaveTextContent('Queued');
		expect(screen.getByText('Job #7')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Testing…' })).toBeDisabled();

		await rerender({ test: { kind: 'tracking', jobId: 7, job: job(), phase: 'running' } });
		expect(screen.getByTestId('claude-status')).toHaveTextContent('Running');

		await rerender({
			test: { kind: 'tracking', jobId: 7, job: job({ attempts: 2 }), phase: 'retrying' }
		});
		expect(screen.getByTestId('claude-status')).toHaveTextContent('Retrying');
		expect(screen.getByText('Attempt 2 of 3')).toBeInTheDocument();

		await rerender({
			test: { kind: 'tracking', jobId: 7, job: job(), phase: 'deferred' },
			queuePausedUntil: '2026-10-22T00:15:00Z'
		});
		expect(screen.getByTestId('claude-status')).toHaveTextContent('Waiting (usage limit)');
		expect(screen.getByText(/^Queue paused until/)).toBeInTheDocument();

		await rerender({
			test: {
				kind: 'tracking',
				jobId: 7,
				job: job({ status: 'succeeded', result: { text: 'pong' } }),
				phase: 'succeeded'
			},
			testing: false,
			queuePausedUntil: null
		});
		expect(screen.getByTestId('claude-status')).toHaveTextContent('Succeeded');
		expect(screen.getByTestId('claude-reply')).toHaveTextContent('pong');
		expect(screen.getByRole('button', { name: 'Test again' })).toBeEnabled();

		await rerender({
			test: {
				kind: 'tracking',
				jobId: 7,
				job: job({ status: 'failed', last_error: 'boom' }),
				phase: 'failed'
			}
		});
		expect(screen.getByTestId('claude-status')).toHaveTextContent('Failed');
		expect(screen.getByRole('button', { name: 'Test Claude' })).toBeEnabled();
	});

	it('ClaudeCard errors and failure text', async () => {
		const { rerender } = renderClaude({ kind: 'start-failed', reason: 'debug-build' });
		expect(screen.getByRole('alert')).toHaveTextContent(
			'The test route needs a debug API build (cargo run).'
		);
		expect(screen.getByRole('button', { name: 'Test Claude' })).toBeEnabled();

		await rerender({ test: { kind: 'start-failed', reason: 'other' } });
		expect(screen.getByRole('alert')).toHaveTextContent("Couldn't start the test prompt.");

		const hostile = 'claude failed: <img src=x onerror=alert(1)>';
		await rerender({
			test: {
				kind: 'tracking',
				jobId: 7,
				job: job({ status: 'failed', last_error: hostile }),
				phase: 'failed'
			}
		});
		const error = screen.getByTestId('claude-error');
		expect(error.textContent).toBe(hostile);
		expect(error).toHaveClass('max-h-40');
		expect(document.querySelector('img')).toBeNull();
	});
});

describe('AgentsCard', () => {
	it('AgentsCard states', async () => {
		const user = userEvent.setup();
		const ontoggle = vi.fn();
		const { rerender } = render(AgentsCard, {
			killSwitch: { kind: 'loading' },
			saving: false,
			ontoggle
		});
		expect(screen.getByTestId('agents-loading')).toBeInTheDocument();
		expect(screen.queryByRole('button')).toBeNull();

		await rerender({ killSwitch: { kind: 'error' } });
		expect(screen.getByText("Couldn't load the agent status.")).toBeInTheDocument();
		expect(screen.queryByRole('button')).toBeNull();

		const at = '2026-10-22T00:00:00Z';
		await rerender({ killSwitch: { kind: 'loaded', state: { paused: false, updated_at: at } } });
		expect(screen.getByTestId('agents-status')).toHaveTextContent('Running');
		expect(screen.getByText('Agents can start and continue runs.')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Pause agents' }));
		expect(ontoggle).toHaveBeenLastCalledWith(true);

		await rerender({ saving: true });
		expect(screen.getByRole('button', { name: 'Pausing…' })).toBeDisabled();

		await rerender({
			killSwitch: { kind: 'loaded', state: { paused: true, updated_at: at } },
			saving: false
		});
		expect(screen.getByTestId('agents-status')).toHaveTextContent('Paused');
		expect(
			screen.getByText(
				'Agents are paused. Queued and new runs wait; running runs stop after their current step.'
			)
		).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Resume agents' }));
		expect(ontoggle).toHaveBeenLastCalledWith(false);

		await rerender({ saving: true });
		expect(screen.getByRole('button', { name: 'Resuming…' })).toBeDisabled();
	});
});

const PHRASE_LABEL = 'Type "delete all data" to confirm';

describe('DeleteAllDataDialog', () => {
	it('DeleteAllDataDialog confirmation', async () => {
		const user = userEvent.setup();
		const submit = vi.fn(async () => true);
		const { rerender } = render(DeleteAllDataDialog, { open: true, submit });
		expect(screen.getByRole('dialog', { name: 'Delete all data?' })).toBeInTheDocument();
		const confirm = () => screen.getByRole('button', { name: 'Delete all data' });
		expect(confirm()).toBeDisabled();

		// Only the exact phrase enables it.
		const input = screen.getByLabelText(PHRASE_LABEL);
		for (const wrong of ['Delete all data', 'delete all data ', 'delete all']) {
			await user.clear(input);
			await user.type(input, wrong);
			expect(confirm()).toBeDisabled();
		}

		// Cancel closes and clears the input.
		await user.click(screen.getByRole('button', { name: 'Cancel' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(submit).not.toHaveBeenCalled();
		await rerender({ open: true });
		expect(screen.getByLabelText(PHRASE_LABEL)).toHaveValue('');
		expect(confirm()).toBeDisabled();
	});

	it('sends the phrase, closes on success and stays open on failure', async () => {
		const user = userEvent.setup();
		let finish!: (ok: boolean) => void;
		const submit = vi.fn(() => new Promise<boolean>((resolve) => (finish = resolve)));
		render(DeleteAllDataDialog, { open: true, submit });

		const input = screen.getByLabelText(PHRASE_LABEL);
		await user.type(input, 'delete all data');
		await user.click(screen.getByRole('button', { name: 'Delete all data' }));
		expect(submit).toHaveBeenCalledWith('delete all data');
		expect(screen.getByRole('button', { name: 'Deleting…' })).toBeDisabled();

		finish(false);
		await waitFor(() =>
			expect(screen.getByRole('button', { name: 'Delete all data' })).toBeEnabled()
		);
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(input).toHaveValue('delete all data');

		await user.click(screen.getByRole('button', { name: 'Delete all data' }));
		finish(true);
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
	});
});

describe('DataCard', () => {
	it('opens the dialog and shows the outcome toasts', async () => {
		const user = userEvent.setup();
		let google: 'revoked' | 'not_revoked' = 'not_revoked';
		const ondelete = vi.fn(async () => ({ google }));
		render(DataCard, { ondelete });
		expect(screen.queryByRole('dialog')).toBeNull();

		await user.click(screen.getByRole('button', { name: 'Delete all data…' }));
		await user.type(screen.getByLabelText(PHRASE_LABEL), 'delete all data');
		await user.click(screen.getByRole('button', { name: 'Delete all data' }));
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(ondelete).toHaveBeenCalledWith('delete all data');
		expect(toast.items.map((t) => [t.tone, t.title, t.description, t.duration])).toEqual([
			['success', 'All data deleted', undefined, 5000],
			[
				'warning',
				"Google didn't confirm the revocation",
				'Remove Waypoint at myaccount.google.com/permissions.',
				0
			]
		]);

		// The input was cleared, and a revoked grant gets no warning.
		toast.clear();
		google = 'revoked';
		await user.click(screen.getByRole('button', { name: 'Delete all data…' }));
		const input = screen.getByLabelText(PHRASE_LABEL);
		expect(input).toHaveValue('');
		await user.type(input, 'delete all data');
		await user.click(screen.getByRole('button', { name: 'Delete all data' }));
		await waitFor(() => expect(toast.items.map((t) => t.tone)).toEqual(['success']));
	});
});
