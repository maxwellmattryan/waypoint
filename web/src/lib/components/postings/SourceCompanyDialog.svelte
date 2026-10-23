<script lang="ts" module>
	import type { ApiClient } from '#lib/api';

	/** What the dialog queued: the board's `source_board` job and its slug. */
	export interface QueuedBoard {
		jobId: number;
		slug: string;
		/** Whether a fetch of this board was already queued or running. */
		reused: boolean;
	}

	export interface SourceCompanyDialogProps {
		/** Bindable. */
		open?: boolean;
		client: ApiClient;
		/** Called after a 202; the dialog has closed by then. */
		onqueued: (queued: QueuedBoard) => void;
	}
</script>

<script lang="ts">
	import type { Source } from '#lib/api';
	import { Button, Dialog, Input, Select, type SelectOption } from '#lib/components/ui';
	import { SOURCE_LABELS, SOURCES, sourceBoard } from '#lib/postings/api.js';

	let { open = $bindable(false), client, onqueued }: SourceCompanyDialogProps = $props();

	// The API's own rules and messages (api/src/sourcing/mod.rs).
	const REQUIRED = 'Required';
	const BAD_SLUG = 'Must be 1–100 letters, digits, hyphens or underscores';
	const SLUG = /^[A-Za-z0-9_-]{1,100}$/;
	const NETWORK = "Couldn't queue the fetch. Check that the API is running.";

	const options: SelectOption[] = SOURCES.map((value) => ({ value, label: SOURCE_LABELS[value] }));

	const formId = $props.id();
	let source = $state<Source | ''>('');
	let slug = $state('');
	let errors = $state<{ source?: string; board?: string }>({});
	let failure = $state<string | null>(null);
	let busy = $state(false);
	/** Bumped on every open and close, so a request sent before it is ignored. */
	let generation = 0;

	function reset() {
		generation++;
		source = '';
		slug = '';
		errors = {};
		failure = null;
		busy = false;
	}

	// A fresh start on every open; closing drops any request in flight.
	$effect(() => {
		if (open) return reset;
	});

	/** The client-side checks, with the server's messages. */
	function check(board: string): { source?: string; board?: string } {
		const found: { source?: string; board?: string } = {};
		if (!source) found.source = REQUIRED;
		if (board === '') found.board = REQUIRED;
		else if (!SLUG.test(board)) found.board = BAD_SLUG;
		return found;
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		const board = slug.trim();
		failure = null;
		errors = check(board);
		if (errors.source || errors.board || !source) return;
		const mine = generation;
		busy = true;
		const result = await sourceBoard(client, { source, board });
		if (mine !== generation) return;
		busy = false;
		switch (result.kind) {
			case 'ok':
				open = false;
				onqueued({ jobId: result.data.job.id, slug: board, reused: result.data.reused });
				return;
			case 'invalid': {
				const mapped: { source?: string; board?: string } = {};
				for (const error of result.errors) {
					if (error.field === 'source') mapped.source = error.message;
					else if (error.field === 'board') mapped.board = error.message;
				}
				if (!mapped.source && !mapped.board) failure = NETWORK;
				errors = mapped;
				return;
			}
			default:
				failure = NETWORK;
		}
	}
</script>

<Dialog
	bind:open
	title="Source a company"
	description="Waypoint reads the company's public job board and keeps its open roles here."
>
	<form
		id={formId}
		novalidate
		onsubmit={submit}
		aria-label="Source a company"
		class="flex flex-col gap-4"
	>
		<Select
			label="Board"
			placeholder="Choose a board"
			{options}
			bind:value={source}
			disabled={busy}
			error={errors.source}
			data-autofocus
		/>
		<Input
			label="Board slug"
			hint="The last part of the board URL, e.g. boards.greenhouse.io/example-labs → example-labs"
			autocomplete="off"
			spellcheck="false"
			bind:value={slug}
			disabled={busy}
			error={errors.board}
		/>
		{#if failure}
			<p class="text-sm text-danger" role="alert">{failure}</p>
		{/if}
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button type="submit" form={formId} disabled={busy}>
			{busy ? 'Queuing…' : 'Source'}
		</Button>
	{/snippet}
</Dialog>
