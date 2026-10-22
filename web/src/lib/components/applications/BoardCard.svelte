<script lang="ts">
	import { resolve } from '$app/paths';
	import type { Application, ApplicationStatus } from '#lib/api';
	import { formatDate } from '#lib/applications/format.js';
	import { STATUSES, statusLabel } from '#lib/applications/statuses.js';
	import { Card, IconButton, Link, Menu } from '#lib/components/ui';

	let {
		application,
		pending = false,
		onmove,
		ondragstart,
		ondragend
	}: {
		application: Application;
		/** Its move request is in flight: busy, not draggable, Move disabled. */
		pending?: boolean;
		/** Called with a status picked from the Move menu. */
		onmove: (status: ApplicationStatus) => void;
		ondragstart: (event: DragEvent) => void;
		ondragend: () => void;
	} = $props();

	const href = $derived(resolve('/applications/[id]', { id: String(application.id) }));
	const name = $derived(`${application.company} · ${application.title}`);
	const targets = $derived(
		STATUSES.filter((status) => status !== application.status).map((status) => ({
			value: status,
			label: statusLabel[status]
		}))
	);
</script>

<Card draggable={!pending} aria-busy={pending} {ondragstart} {ondragend}>
	<div class="flex items-start justify-between gap-2">
		<div class="flex min-w-0 flex-col gap-1">
			<!-- Not draggable itself, so a drag that starts on the title moves the card, not the URL. -->
			<!-- Company and title on their own lines; the link's name reads "company · title". -->
			<Link {href} draggable="false" aria-label={name} class="flex min-w-0 flex-col text-sm">
				<span class="truncate font-semibold">{application.company}</span>
				<span class="truncate text-fg-muted">{application.title}</span>
			</Link>
			<p class="flex flex-wrap gap-x-2 text-xs text-fg-muted">
				{#if application.location}
					<span class="truncate">{application.location}</span>
				{/if}
				<span class="whitespace-nowrap">Updated {formatDate(application.updated_at)}</span>
			</p>
		</div>
		<Menu
			label="Move to"
			heading="Move to"
			items={targets}
			onselect={(value) => onmove(value as ApplicationStatus)}
		>
			{#snippet trigger(t)}
				<IconButton
					{...t}
					size="sm"
					label="Move {name}"
					disabled={pending}
					data-move-trigger={application.id}
				>
					<svg class="size-4" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
						<circle cx="3" cy="8" r="1.5" />
						<circle cx="8" cy="8" r="1.5" />
						<circle cx="13" cy="8" r="1.5" />
					</svg>
				</IconButton>
			{/snippet}
		</Menu>
	</div>
</Card>
