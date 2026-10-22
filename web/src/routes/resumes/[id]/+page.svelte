<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { fade } from 'svelte/transition';
	import { createApiClient, type ResumeVersionDetail } from '#lib/api';
	import { formatDate } from '#lib/applications/format.js';
	import { deleteResume, getResume, pdfUrl } from '#lib/resumes/api.js';
	import { formatBytes, usedBy } from '#lib/resumes/format.js';
	import { Badge, Button, Card, Dialog, EmptyState, Link, Skeleton, toast } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';

	type View =
		| { kind: 'loading' }
		| { kind: 'not-found' }
		| { kind: 'error' }
		| { kind: 'loaded'; version: ResumeVersionDetail };

	const client = createApiClient();
	const listHref = resolve('/resumes');

	/** The route's id as a positive integer, or `null` for anything else (no request is made). */
	const id = $derived(/^[1-9]\d*$/.test(page.params.id ?? '') ? Number(page.params.id) : null);

	let view = $state<View>({ kind: 'loading' });
	let confirmingDelete = $state(false);
	let deleting = $state(false);

	async function load(target: number | null) {
		if (target === null || !Number.isSafeInteger(target)) {
			view = { kind: 'not-found' };
			return;
		}
		view = { kind: 'loading' };
		const result = await getResume(client, target);
		if (target !== id) return;
		if (result.kind === 'ok') view = { kind: 'loaded', version: result.data };
		else view = { kind: result.kind === 'not-found' ? 'not-found' : 'error' };
	}

	$effect(() => {
		void load(id);
	});

	async function remove(version: ResumeVersionDetail) {
		deleting = true;
		const result = await deleteResume(client, version.id);
		deleting = false;
		if (result.kind === 'ok') {
			confirmingDelete = false;
			toast.success('Resume deleted');
			await goto(listHref);
		} else if (result.kind === 'conflict') {
			confirmingDelete = false;
			toast.error('This resume is linked to an application');
			void load(id);
		} else {
			toast.error("Couldn't delete the resume");
		}
	}
</script>

<svelte:head>
	<title>{view.kind === 'loaded' ? view.version.name : 'Resume'} · Waypoint</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<Link href={listHref} class="self-start text-sm">All resumes</Link>

	{#if view.kind === 'loading'}
		<div class="flex flex-col gap-4" aria-busy="true" aria-label="Loading resume">
			<Skeleton class="h-8 w-64" />
			<Skeleton shape="text" class="w-40" />
			<Skeleton class="h-64" />
		</div>
	{:else if view.kind === 'not-found'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Resume not found"
				description="It may have been deleted, or the link is wrong."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => goto(listHref)}>Back to resumes</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load this resume"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => load(id)}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		{@const version = view.version}
		{@const linked = version.application_count > 0}
		<div class="flex flex-col gap-8" in:fade={{ duration: motionDuration('base') }}>
			<header class="flex flex-wrap items-start justify-between gap-4">
				<div class="flex min-w-0 flex-col gap-2">
					<h1 class="text-2xl font-semibold break-words text-fg">{version.name}</h1>
					<div class="flex flex-wrap items-center gap-2">
						<p class="text-base break-all text-fg-muted">{version.filename}</p>
						{#if !version.has_text}
							<Badge tone="warning">No text</Badge>
						{/if}
					</div>
				</div>
				<div class="flex flex-col items-end gap-2">
					<div class="flex items-center gap-4">
						<Link href={pdfUrl(version.id)} external class="text-sm">Open PDF</Link>
						<Button
							variant="danger"
							disabled={linked}
							aria-describedby={linked ? 'delete-blocked' : undefined}
							onclick={() => (confirmingDelete = true)}
						>
							Delete
						</Button>
					</div>
					{#if linked}
						<p id="delete-blocked" class="text-xs text-fg-muted">
							{usedBy(version.application_count)}; unlink it before deleting.
						</p>
					{/if}
				</div>
			</header>

			<Card>
				<div class="flex flex-col gap-6">
					<dl class="grid gap-x-8 gap-y-4 text-sm sm:grid-cols-2">
						{#snippet item(label: string, value: string)}
							<div class="flex flex-col gap-1">
								<dt class="text-fg-muted">{label}</dt>
								<dd class="break-words text-fg">{value}</dd>
							</div>
						{/snippet}
						{@render item('Filename', version.filename)}
						{@render item('Size', formatBytes(version.size_bytes))}
						{@render item('Uploaded', formatDate(version.created_at))}
						{@render item('Applications', usedBy(version.application_count))}
					</dl>

					<section
						class="flex flex-col gap-2 border-t border-border pt-4"
						aria-labelledby="text-heading"
					>
						<h2 id="text-heading" class="text-sm font-semibold text-fg">Text</h2>
						{#if version.text}
							<p class="text-sm break-words whitespace-pre-wrap text-fg">{version.text}</p>
						{:else}
							<p class="text-sm text-fg-subtle">No text could be extracted from this PDF.</p>
						{/if}
					</section>
				</div>
			</Card>
		</div>

		<Dialog
			bind:open={confirmingDelete}
			title="Delete resume?"
			description="{version.name} ({version.filename}) will be removed permanently."
			size="sm"
		>
			{#snippet footer()}
				<Button variant="secondary" onclick={() => (confirmingDelete = false)}>Cancel</Button>
				<Button variant="danger" disabled={deleting} onclick={() => remove(version)}>
					{deleting ? 'Deleting…' : 'Delete'}
				</Button>
			{/snippet}
		</Dialog>
	{/if}
</div>
