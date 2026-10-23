<script lang="ts" module>
	import type { Memory, MemoryInput } from '#lib/api';
	import type { Result } from '#lib/memories/api.js';

	export interface MemoryFormDialogProps {
		/** Bindable. */
		open?: boolean;
		/** The memory being edited; `null` or omitted adds a new one. Read when the dialog opens. */
		memory?: Memory | null;
		/** Sends the input: create or update. */
		save: (input: MemoryInput) => Promise<Result<Memory>>;
		/** Called with the stored memory after a successful save; the dialog is closed by then. */
		onsaved?: (memory: Memory, wasNew: boolean) => void;
	}
</script>

<script lang="ts">
	import { untrack } from 'svelte';
	import type { MemoryKind, MemoryTopic } from '#lib/api';
	import { Button, Dialog, Select, TagInput, Textarea } from '#lib/components/ui';
	import { KIND_LABELS, TOPICS_BY_KIND, TOPIC_LABELS, topicAllowed } from '#lib/memories/topics.js';

	let { open = $bindable(false), memory = null, save, onsaved }: MemoryFormDialogProps = $props();

	type FieldName = 'kind' | 'topic' | 'content' | 'tags';
	const FIELD_NAMES: readonly string[] = ['kind', 'topic', 'content', 'tags'];

	const formId = $props.id();
	let kind = $state<MemoryKind>('fact');
	let topic = $state<MemoryTopic>('position');
	let content = $state('');
	let tags = $state<string[]>([]);
	let submitting = $state(false);
	let errors = $state<Partial<Record<FieldName, string>>>({});
	/** An error with no field of its own, shown above the actions. */
	let failure = $state<string | null>(null);

	const editing = $derived(memory !== null);
	const kindOptions = Object.entries(KIND_LABELS).map(([value, label]) => ({ value, label }));
	const topicOptions = $derived(
		TOPICS_BY_KIND[kind].map((value) => ({ value, label: TOPIC_LABELS[value] }))
	);

	function fill() {
		kind = memory?.kind ?? 'fact';
		topic = memory && topicAllowed(memory.kind, memory.topic) ? memory.topic : TOPICS_BY_KIND[kind][0];
		content = memory?.content ?? '';
		tags = [...(memory?.tags ?? [])];
		errors = {};
		failure = null;
	}

	// Fill the form each time the dialog opens: from the memory when editing, blank otherwise.
	$effect.pre(() => {
		if (open) untrack(fill);
	});

	function changeKind(next: MemoryKind) {
		kind = next;
		topic = TOPICS_BY_KIND[next][0];
		errors = { ...errors, topic: undefined };
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (submitting) return;
		submitting = true;
		errors = {};
		failure = null;
		const wasNew = !editing;
		const result = await save({ kind, topic, content, tags });
		submitting = false;
		switch (result.kind) {
			case 'ok':
				open = false;
				onsaved?.(result.data, wasNew);
				break;
			case 'invalid': {
				const placed: Partial<Record<FieldName, string>> = {};
				const other: string[] = [];
				for (const error of result.errors) {
					if (FIELD_NAMES.includes(error.field)) placed[error.field as FieldName] ??= error.message;
					else other.push(error.message);
				}
				errors = placed;
				if (other.length > 0) failure = other.join(' ');
				break;
			}
			case 'not-found':
				failure = 'This memory was deleted elsewhere.';
				break;
			default:
				failure = "Couldn't save the memory. Check that the API is running, then try again.";
		}
	}
</script>

<Dialog bind:open title={editing ? 'Edit memory' : 'Add memory'} size="lg">
	<form id={formId} class="flex flex-col gap-4" novalidate onsubmit={submit}>
		<div class="grid gap-4 sm:grid-cols-2">
			<Select
				label="Kind"
				options={kindOptions}
				value={kind}
				error={errors.kind}
				disabled={submitting}
				onchange={(event) => changeKind(event.currentTarget.value as MemoryKind)}
			/>
			<Select
				label="Topic"
				options={topicOptions}
				bind:value={() => topic, (next) => (topic = next as MemoryTopic)}
				error={errors.topic}
				disabled={submitting}
			/>
		</div>
		<Textarea
			label="Content"
			rows={4}
			maxlength={5000}
			data-autofocus
			error={errors.content}
			disabled={submitting}
			bind:value={content}
		/>
		<TagInput
			label="Tags"
			hint="Enter or comma adds a tag."
			error={errors.tags}
			disabled={submitting}
			bind:value={tags}
		/>
		{#if failure}
			<p class="text-sm text-danger" role="alert">{failure}</p>
		{/if}
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button type="submit" form={formId} disabled={submitting}>
			{submitting ? 'Saving…' : 'Save'}
		</Button>
	{/snippet}
</Dialog>
