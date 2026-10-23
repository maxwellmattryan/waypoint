<script lang="ts" module>
	import type { EmailReview } from '#lib/review/email.js';

	export interface EmailReviewSummaryProps {
		summary: EmailReview;
	}
</script>

<script lang="ts">
	import { KeyValue, Tag, type KeyValueItem } from '#lib/components/ui';

	/** An email review item, readable: the email, its target, the changes and why it matched. */
	let { summary }: EmailReviewSummaryProps = $props();

	const emailLine = $derived(
		[`"${summary.email.subject}"`, summary.email.sender, summary.email.date]
			.filter((part) => part !== null)
			.join(' · ')
	);

	const items = $derived.by(() => {
		const rows: KeyValueItem[] = [
			{ key: 'Email', value: emailLine },
			{
				key: summary.kind === 'new_application' ? 'New application' : 'Application',
				value: summary.target
			},
			{ key: 'Changes', value: changes }
		];
		if (summary.reasons.length > 0) rows.push({ key: 'Matched on', value: reasons });
		if (summary.candidates.length > 0) rows.push({ key: 'Also possible', value: candidates });
		return rows;
	});
</script>

{#snippet changes()}
	<ul class="flex flex-col gap-1" aria-label="Proposed changes">
		{#each summary.changes as change, i (i)}
			<li>{change}</li>
		{/each}
	</ul>
{/snippet}

{#snippet reasons()}
	<ul class="flex flex-wrap gap-2" aria-label="Match reasons">
		{#each summary.reasons as reason (reason)}
			<li class="flex"><Tag label={reason} /></li>
		{/each}
	</ul>
{/snippet}

{#snippet candidates()}
	<ul class="flex flex-col gap-1" aria-label="Other candidates">
		{#each summary.candidates as candidate, i (i)}
			<li>{candidate}</li>
		{/each}
	</ul>
{/snippet}

<KeyValue {items} />
