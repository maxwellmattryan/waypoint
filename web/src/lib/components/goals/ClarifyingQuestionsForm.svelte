<script lang="ts" module>
	import type { GoalQuestion } from '#lib/api';
	import type { Answer } from '#lib/goals/api.js';

	export interface ClarifyingQuestionsFormProps {
		/** The open questions, by ordinal. */
		questions: GoalQuestion[];
		/** The answers are being sent; both buttons are disabled. */
		busy?: boolean;
		/** Sends one answer per question; a blank one skips it. */
		onsubmit: (answers: Answer[]) => void;
	}

	/** Most characters in one answer, as the API allows. */
	export const MAX_ANSWER = 1000;
</script>

<script lang="ts">
	import { Button, Card, Textarea } from '#lib/components/ui';

	/** The planner's clarifying questions, one textarea each. */
	let { questions, busy = false, onsubmit }: ClarifyingQuestionsFormProps = $props();

	const formId = $props.id();
	let answers = $state<Record<number, string>>({});
	const tooLong = $derived(
		questions.filter((q) => [...(answers[q.id] ?? '').trim()].length > MAX_ANSWER).map((q) => q.id)
	);

	function send(skip: boolean) {
		if (busy || (!skip && tooLong.length > 0)) return;
		onsubmit(
			questions.map((q) => {
				const answer = skip ? '' : (answers[q.id] ?? '').trim();
				return answer ? { question_id: q.id, answer } : { question_id: q.id };
			})
		);
	}

	function submit(event: SubmitEvent) {
		event.preventDefault();
		send(false);
	}
</script>

<Card class="flex flex-col gap-4">
	<div class="flex flex-col gap-1">
		<h2 class="text-lg font-semibold text-fg">A few questions first</h2>
		<p class="text-sm text-fg-muted">
			Answer what you can; a blank answer skips the question. Waypoint plans once you send them.
		</p>
	</div>
	<form id={formId} class="flex flex-col gap-4" aria-label="Clarifying questions" onsubmit={submit} novalidate>
		{#each questions as question (question.id)}
			<Textarea
				label={question.question}
				rows={2}
				value={answers[question.id] ?? ''}
				oninput={(event) => (answers[question.id] = event.currentTarget.value)}
				disabled={busy}
				error={tooLong.includes(question.id) ? `At most ${MAX_ANSWER} characters` : undefined}
			/>
		{/each}
	</form>
	<div class="flex flex-wrap justify-end gap-2">
		<Button variant="secondary" disabled={busy} onclick={() => send(true)}>
			Skip and plan anyway
		</Button>
		<Button type="submit" form={formId} disabled={busy || tooLong.length > 0}>
			{busy ? 'Sending…' : 'Submit answers'}
		</Button>
	</div>
</Card>
