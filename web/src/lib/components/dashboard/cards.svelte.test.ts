import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { GoogleStatus, Job } from '#lib/api';
import type { TestState } from '#lib/dashboard/dashboard.svelte.js';
import ClaudeCard from './ClaudeCard.svelte';
import GoogleCard from './GoogleCard.svelte';
import SystemCard from './SystemCard.svelte';

afterEach(() => vi.restoreAllMocks());

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
