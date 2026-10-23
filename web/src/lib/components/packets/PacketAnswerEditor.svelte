<script lang="ts" module>
	import type { ApplicationPacket, PendingAction } from '#lib/api';
	import type { ApprovalInput, Result } from '#lib/packets/api.js';
	import type { ResumesState } from '#lib/packets/packets.svelte.js';

	export interface PacketAnswerEditorProps {
		/** The packet whose draft is edited. Read when the form opens. */
		packet: ApplicationPacket;
		/** The resumes the Select offers. */
		resumes: ResumesState;
		/** Loads the resumes again after a failure. */
		onretry: () => void;
		/** Approves the edited input; a 422's message shows above the buttons. */
		onsubmit: (input: ApprovalInput) => Promise<Result<PendingAction> | null>;
		oncancel: () => void;
	}
</script>

<script lang="ts">
	import { untrack } from 'svelte';
	import type { PacketQuestion } from '#lib/api';
	import { Button, Select, Textarea } from '#lib/components/ui';
	import { MAX_ANSWER } from '#lib/packets/api.js';

	/**
	 * The packet's edit form: a resume Select and one Textarea per question, prefilled from the
	 * draft. "Approve with edits" sends the trimmed answers (a blank one as `null`).
	 */
	let { packet, resumes, onretry, onsubmit, oncancel }: PacketAnswerEditorProps = $props();

	const TOO_LONG = 'At most 1,500 characters';
	const formId = $props.id();

	const draft = untrack(() => packet);
	let resume = $state(draft.resume_version_id === null ? '' : String(draft.resume_version_id));
	let answers = $state<Record<string, string>>(
		Object.fromEntries(draft.answers.map((answer) => [answer.key, answer.answer ?? '']))
	);
	let submitting = $state(false);
	let resumeError = $state<string | undefined>(undefined);
	let answerErrors = $state<Record<string, string>>({});
	/** The server's refusal, shown above the buttons. */
	let failure = $state<string | null>(null);

	const options = $derived(
		resumes.kind === 'loaded'
			? resumes.resumes.map((r) => ({ value: String(r.id), label: r.name }))
			: []
	);
	// Preselect the draft's resume only while it is one of the user's resumes.
	const value = $derived(options.some((option) => option.value === resume) ? resume : '');

	function check(): boolean {
		resumeError = value === '' ? 'Choose a resume' : undefined;
		const errors: Record<string, string> = {};
		for (const [key, text] of Object.entries(answers)) {
			// Characters, as the API counts them, not UTF-16 units.
			if ([...text.trim()].length > MAX_ANSWER) errors[key] = TOO_LONG;
		}
		answerErrors = errors;
		return resumeError === undefined && Object.keys(errors).length === 0;
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (submitting) return;
		failure = null;
		if (!check()) return;
		const trimmed = {} as Record<PacketQuestion, string | null>;
		for (const answer of draft.answers) {
			const text = (answers[answer.key] ?? '').trim();
			trimmed[answer.key] = text === '' ? null : text;
		}
		submitting = true;
		const result = await onsubmit({
			packet_id: draft.id,
			resume_version_id: Number(value),
			answers: trimmed
		});
		submitting = false;
		if (result?.kind === 'invalid') failure = result.errors.map((e) => e.message).join(' ');
	}
</script>

<form id={formId} class="flex flex-col gap-6" novalidate onsubmit={submit}>
	<div class="flex flex-col gap-2">
		<Select
			label="Resume"
			placeholder="Choose a resume"
			{options}
			bind:value={() => value, (next) => (resume = next ?? '')}
			hint={resumes.kind === 'loading' || resumes.kind === 'idle' ? 'Loading resumes…' : undefined}
			error={resumeError}
			disabled={submitting || resumes.kind !== 'loaded'}
		/>
		{#if resumes.kind === 'error'}
			<div class="flex flex-wrap items-center gap-2">
				<p class="text-sm text-danger" role="alert">Couldn't load resumes</p>
				<Button variant="secondary" size="sm" onclick={onretry}>Try again</Button>
			</div>
		{/if}
	</div>
	{#each draft.answers as answer (answer.key)}
		<Textarea
			label={answer.question}
			rows={4}
			error={answerErrors[answer.key]}
			disabled={submitting}
			bind:value={answers[answer.key]}
		/>
	{/each}
	{#if failure}
		<p class="text-sm text-danger" role="alert">{failure}</p>
	{/if}
	<div class="flex flex-wrap gap-2">
		<Button type="submit" disabled={submitting}>
			{submitting ? 'Approving…' : 'Approve with edits'}
		</Button>
		<Button variant="secondary" disabled={submitting} onclick={oncancel}>Cancel</Button>
	</div>
</form>
