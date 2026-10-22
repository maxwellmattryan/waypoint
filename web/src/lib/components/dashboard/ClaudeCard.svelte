<script lang="ts" module>
	import type { BadgeTone } from '#lib/components/ui';
	import type { TestState } from '#lib/dashboard/dashboard.svelte.js';
	import type { JobPhase } from '#lib/dashboard/job.js';

	export interface ClaudeCardProps {
		test: TestState;
		/** Whether a test is starting or its job is still in flight. */
		testing: boolean;
		/** When the paused job queue resumes; `null` while it isn't paused. */
		queuePausedUntil: string | null;
		ontest: () => void;
	}

	const phases: Record<JobPhase, { tone: BadgeTone; label: string }> = {
		queued: { tone: 'neutral', label: 'Queued' },
		running: { tone: 'info', label: 'Running' },
		retrying: { tone: 'warning', label: 'Retrying' },
		deferred: { tone: 'warning', label: 'Waiting (usage limit)' },
		succeeded: { tone: 'success', label: 'Succeeded' },
		failed: { tone: 'danger', label: 'Failed' }
	};
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { Badge, Button } from '#lib/components/ui';
	import { resultText } from '#lib/dashboard/job.js';
	import { motionDuration } from '#lib/motion.js';
	import DashboardCard from './DashboardCard.svelte';

	let { test, testing, queuePausedUntil, ontest }: ClaudeCardProps = $props();

	const tracked = $derived(test.kind === 'tracking' ? test : null);
	const label = $derived(
		testing ? 'Testing…' : tracked?.phase === 'succeeded' ? 'Test again' : 'Test Claude'
	);
</script>

<DashboardCard title="Claude">
	{#snippet status()}
		{#if tracked}
			{#key tracked.phase}
				<span in:fade={{ duration: motionDuration('base') }}>
					<Badge tone={phases[tracked.phase].tone} data-testid="claude-status">
						{phases[tracked.phase].label}
					</Badge>
				</span>
			{/key}
		{/if}
	{/snippet}

	<p class="text-sm text-fg-muted">
		Send a test prompt through <code class="font-mono text-xs">claude -p</code> on your subscription.
	</p>

	{#if tracked}
		<div class="flex flex-col gap-2 text-sm text-fg-muted" aria-live="polite">
			<p>Job #{tracked.jobId}</p>
			{#if tracked.phase === 'retrying' && tracked.job}
				<p>Attempt {tracked.job.attempts} of {tracked.job.max_attempts}</p>
			{/if}
			{#if testing && queuePausedUntil}
				<p>Queue paused until {new Date(queuePausedUntil).toLocaleTimeString()}</p>
			{/if}
		</div>
		{#if tracked.phase === 'succeeded' && tracked.job}
			<pre
				class="rounded-md bg-surface-raised p-2 font-mono text-sm whitespace-pre-wrap text-fg"
				data-testid="claude-reply"
				in:fade={{ duration: motionDuration('base') }}>{resultText(tracked.job) ?? ''}</pre>
		{:else if tracked.phase === 'failed' && tracked.job?.last_error}
			<pre
				class="max-h-40 overflow-auto rounded-md bg-danger-soft p-2 font-mono text-xs whitespace-pre-wrap text-danger-soft-fg"
				data-testid="claude-error"
				in:fade={{ duration: motionDuration('base') }}>{tracked.job.last_error}</pre>
		{/if}
	{:else if test.kind === 'start-failed'}
		<p class="text-sm text-danger" role="alert">
			{#if test.reason === 'debug-build'}
				The test route needs a debug API build (<code class="font-mono text-xs">cargo run</code>).
			{:else}
				Couldn't start the test prompt.
			{/if}
		</p>
	{/if}

	<div><Button onclick={ontest} disabled={testing}>{label}</Button></div>
</DashboardCard>
