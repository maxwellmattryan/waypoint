<script lang="ts" module>
	import type { ApiClient, Capture, connectEvents as ConnectEvents } from '#lib/api';

	export interface CaptureDialogProps {
		/** Bindable. */
		open?: boolean;
		client: ApiClient;
		/** Called with the capture; the dialog has closed by then. */
		oncaptured: (capture: Capture) => void;
		/** Called with the typed URL after "Add by hand"; the dialog has closed by then. */
		onmanual: (url: string) => void;
		/** Replaces the real event stream, for tests. */
		connectEvents?: typeof ConnectEvents;
	}
</script>

<script lang="ts">
	import { fade } from 'svelte/transition';
	import { isCapture, startCapture, watchJob } from '#lib/applications/capture.js';
	import type { JobPhase } from '#lib/dashboard/job.js';
	import { Button, Dialog, Input } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';

	let {
		open = $bindable(false),
		client,
		oncaptured,
		onmanual,
		connectEvents
	}: CaptureDialogProps = $props();

	type Step =
		| { kind: 'idle' }
		| { kind: 'capturing' }
		| { kind: 'waiting'; phase: JobPhase }
		| { kind: 'error'; message: string }
		| { kind: 'job-failed' };

	const FAILED_JOB = "Claude couldn't read this page. Try again, or add the details by hand.";
	const NETWORK = "Couldn't capture the page. Check that the API is running.";

	const formId = $props.id();
	let url = $state('');
	let step = $state<Step>({ kind: 'idle' });
	let stopWatching: (() => void) | null = null;
	/** Bumped on every open and close, so a capture started before it is ignored. */
	let generation = 0;

	const busy = $derived(step.kind === 'capturing' || step.kind === 'waiting');
	const status = $derived.by(() => {
		if (step.kind === 'capturing') return 'Reading the page…';
		if (step.kind !== 'waiting') return '';
		return step.phase === 'deferred'
			? "Claude's usage limit was hit. This continues when it resets."
			: 'Asking Claude to read the posting…';
	});
	const error = $derived(
		step.kind === 'error' ? step.message : step.kind === 'job-failed' ? FAILED_JOB : undefined
	);

	function reset() {
		generation++;
		stopWatching?.();
		stopWatching = null;
		url = '';
		step = { kind: 'idle' };
	}

	// A fresh start on every open; closing stops watching any pending job.
	$effect(() => {
		if (open) return reset;
	});

	function finish(capture: Capture) {
		open = false;
		oncaptured(capture);
	}

	function manual() {
		const typed = url;
		open = false;
		onmanual(typed);
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		if (url.trim() === '') {
			step = { kind: 'error', message: 'Required' };
			return;
		}
		const mine = generation;
		step = { kind: 'capturing' };
		const started = await startCapture(client, url);
		if (mine !== generation) return;
		switch (started.kind) {
			case 'captured':
				finish(started.capture);
				return;
			case 'invalid':
				step = {
					kind: 'error',
					message: started.errors.map((e) => e.message).join(' ') || NETWORK
				};
				return;
			case 'unreadable':
				step = { kind: 'error', message: started.message };
				return;
			case 'failed':
				step = { kind: 'error', message: NETWORK };
				return;
			case 'queued':
				step = { kind: 'waiting', phase: 'queued' };
				stopWatching = watchJob(started.jobId, { client, connectEvents }, (phase, job) => {
					if (mine !== generation) return;
					if (phase === 'succeeded') {
						stopWatching = null;
						if (isCapture(job.result)) finish(job.result);
						else step = { kind: 'job-failed' };
					} else if (phase === 'failed') {
						stopWatching = null;
						step = { kind: 'job-failed' };
					} else {
						step = { kind: 'waiting', phase };
					}
				});
		}
	}
</script>

<Dialog
	bind:open
	title="Capture from URL"
	description="Paste a job posting link. Waypoint reads the page and fills in the details for you to check."
>
	<form id={formId} novalidate onsubmit={submit} aria-label="Capture from URL" class="flex flex-col gap-2">
		<Input
			label="Job posting URL"
			type="url"
			placeholder="https://"
			data-autofocus
			bind:value={url}
			disabled={busy}
			{error}
		/>
		<div class="grid min-h-5 text-sm text-fg-muted" role="status" aria-live="polite">
			{#key status}
				{#if status}
					<p
						class="col-start-1 row-start-1"
						in:fade={{ duration: motionDuration('fast') }}
						out:fade={{ duration: motionDuration('fast') }}
					>
						{status}
					</p>
				{/if}
			{/key}
		</div>
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		{#if step.kind === 'job-failed'}
			<Button variant="secondary" onclick={manual}>Add by hand</Button>
		{/if}
		<Button type="submit" form={formId} disabled={busy}>
			{busy ? 'Capturing…' : 'Capture'}
		</Button>
	{/snippet}
</Dialog>
