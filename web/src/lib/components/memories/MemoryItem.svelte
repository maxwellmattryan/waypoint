<script lang="ts" module>
	import type { Memory } from '#lib/api';

	export interface MemoryItemProps {
		memory: Memory;
		/** Confirms a proposed memory; resolves whether it worked. */
		onconfirm: (memory: Memory) => Promise<boolean>;
		onedit: (memory: Memory) => void;
		ondelete: (memory: Memory) => void;
		/** Filters the list by one of the memory's tags. */
		ontag: (tag: string) => void;
		/** Shows the highlight ring, for the memory a link pointed at. */
		highlighted?: boolean;
	}
</script>

<script lang="ts">
	import { formatDate } from '#lib/applications/format.js';
	import { Badge, Button, Card, Link, Tag } from '#lib/components/ui';
	import { TOPIC_LABELS, sourceLine } from '#lib/memories/topics.js';

	let {
		memory,
		onconfirm,
		onedit,
		ondelete,
		ontag,
		highlighted = false
	}: MemoryItemProps = $props();

	let confirming = $state(false);

	const source = $derived(sourceLine(memory));
	const proposed = $derived(memory.status === 'proposed');
	const updated = $derived(memory.updated_at !== memory.created_at);

	async function confirm() {
		if (confirming) return;
		confirming = true;
		try {
			await onconfirm(memory);
		} finally {
			confirming = false;
		}
	}
</script>

<!-- Focusable from script only (`/memories?memory=<id>` moves focus here); the ring marks it. -->
<Card
	id="memory-{memory.id}"
	tabindex={-1}
	data-highlighted={highlighted ? 'true' : undefined}
	class={[
		'transition-shadow focus:outline-none',
		highlighted && 'ring-2 ring-focus ring-offset-2 ring-offset-bg'
	]}
>
	<div class="flex flex-col gap-4">
		<div class="flex items-start justify-between gap-4">
			<div class="flex min-w-0 flex-col gap-1">
				<p class="text-xs font-medium text-fg-muted">{TOPIC_LABELS[memory.topic]}</p>
				<p class="text-sm break-words whitespace-pre-line text-fg">{memory.content}</p>
			</div>
			{#if proposed}
				<Badge tone="warning">Proposed</Badge>
			{/if}
		</div>

		{#if memory.tags.length > 0}
			<ul class="flex flex-wrap gap-1" aria-label="Tags">
				{#each memory.tags as tag (tag)}
					<li class="flex max-w-full">
						<Tag label={tag} onclick={() => ontag(tag)} />
					</li>
				{/each}
			</ul>
		{/if}

		<div class="flex flex-wrap items-center justify-between gap-2">
			<p class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted">
				<span data-testid="source">
					{#if source.href}
						<Link href={source.href}>{source.text}</Link>
					{:else}
						{source.text}
					{/if}
				</span>
				<span>Added {formatDate(memory.created_at)}</span>
				{#if updated}
					<span>Updated {formatDate(memory.updated_at)}</span>
				{/if}
			</p>
			<div class="flex flex-wrap gap-2">
				{#if proposed}
					<Button size="sm" disabled={confirming} onclick={confirm}>Confirm</Button>
				{/if}
				<Button size="sm" variant="secondary" onclick={() => onedit(memory)}>Edit</Button>
				<Button size="sm" variant="ghost" onclick={() => ondelete(memory)}>Delete</Button>
			</div>
		</div>
	</div>
</Card>
