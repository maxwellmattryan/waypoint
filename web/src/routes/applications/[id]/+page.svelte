<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { fade } from 'svelte/transition';
	import {
		createApiClient,
		type Application,
		type ApplicationStatus,
		type ResumeVersion
	} from '#lib/api';
	import {
		deleteApplication,
		getApplication,
		setApplicationResume,
		setApplicationStatus,
		updateApplication
	} from '#lib/applications/api.js';
	import { listResumes } from '#lib/resumes/api.js';
	import ResumeSelect from '#lib/components/resumes/ResumeSelect.svelte';
	import { formatDate, isWebUrl } from '#lib/applications/format.js';
	import { isStatus, statusLabel } from '#lib/applications/statuses.js';
	import ApplicationFormDialog from '#lib/components/applications/ApplicationFormDialog.svelte';
	import StatusBadge from '#lib/components/applications/StatusBadge.svelte';
	import StatusSelect from '#lib/components/applications/StatusSelect.svelte';
	import { Button, Card, Dialog, EmptyState, Link, Skeleton, toast } from '#lib/components/ui';
	import { motionDuration } from '#lib/motion.js';

	type View =
		| { kind: 'loading' }
		| { kind: 'not-found' }
		| { kind: 'error' }
		| { kind: 'loaded'; application: Application };

	const client = createApiClient();
	const listHref = resolve('/applications');

	/** The route's id as a positive integer, or `null` for anything else (no request is made). */
	const id = $derived(/^[1-9]\d*$/.test(page.params.id ?? '') ? Number(page.params.id) : null);

	let view = $state<View>({ kind: 'loading' });
	let status = $state<ApplicationStatus>('sourced');
	let savingStatus = $state(false);
	let editing = $state(false);
	let confirmingDelete = $state(false);
	let deleting = $state(false);

	/** Every resume version for the Resume section; its failure doesn't block the page. */
	let resumes = $state<
		{ kind: 'loading' } | { kind: 'error' } | { kind: 'loaded'; versions: ResumeVersion[] }
	>({ kind: 'loading' });
	let resumeId = $state<number | null>(null);
	let savingResume = $state(false);

	const resumesHref = resolve('/resumes');

	function show(application: Application) {
		view = { kind: 'loaded', application };
		status = application.status;
		resumeId = application.resume_version_id;
	}

	async function loadResumes() {
		const result = await listResumes(client);
		resumes = result.kind === 'ok' ? { kind: 'loaded', versions: result.data } : { kind: 'error' };
	}

	$effect(() => {
		void loadResumes();
	});

	async function changeResume(application: Application, next: number | null) {
		const previous = application.resume_version_id;
		if (next === previous) return;
		savingResume = true;
		const result = await setApplicationResume(client, application.id, next);
		savingResume = false;
		if (result.kind === 'ok') {
			show(result.data);
			toast.success(result.data.resume_version_id === null ? 'Resume cleared' : 'Resume set');
		} else {
			resumeId = previous;
			toast.error("Couldn't change the resume");
		}
	}

	async function load(target: number | null) {
		if (target === null || !Number.isSafeInteger(target)) {
			view = { kind: 'not-found' };
			return;
		}
		view = { kind: 'loading' };
		const result = await getApplication(client, target);
		if (target !== id) return;
		if (result.kind === 'ok') show(result.data);
		else view = { kind: result.kind === 'not-found' ? 'not-found' : 'error' };
	}

	$effect(() => {
		void load(id);
	});

	async function changeStatus(application: Application, next: ApplicationStatus) {
		const previous = application.status;
		if (next === previous) return;
		savingStatus = true;
		const result = await setApplicationStatus(client, application.id, next);
		savingStatus = false;
		if (result.kind === 'ok') {
			show(result.data);
			toast.success(`Status set to ${statusLabel[result.data.status]}`);
		} else {
			status = previous;
			toast.error("Couldn't change the status");
		}
	}

	async function remove(application: Application) {
		deleting = true;
		const result = await deleteApplication(client, application.id);
		deleting = false;
		if (result.kind === 'ok') {
			confirmingDelete = false;
			toast.success('Application deleted');
			await goto(listHref);
		} else {
			toast.error("Couldn't delete the application");
		}
	}
</script>

<svelte:head>
	<title>{view.kind === 'loaded' ? view.application.company : 'Application'} · Waypoint</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<Link href={listHref} class="self-start text-sm">All applications</Link>

	{#if view.kind === 'loading'}
		<div class="flex flex-col gap-4" aria-busy="true" aria-label="Loading application">
			<Skeleton class="h-8 w-64" />
			<Skeleton shape="text" class="w-40" />
			<Skeleton class="h-64" />
		</div>
	{:else if view.kind === 'not-found'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Application not found"
				description="It may have been deleted, or the link is wrong."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => goto(listHref)}>Back to applications</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else if view.kind === 'error'}
		<div in:fade={{ duration: motionDuration('base') }}>
			<EmptyState
				title="Couldn't load this application"
				description="Check that the API is running, then try again."
			>
				{#snippet action()}
					<Button variant="secondary" onclick={() => load(id)}>Try again</Button>
				{/snippet}
			</EmptyState>
		</div>
	{:else}
		{@const application = view.application}
		<div class="flex flex-col gap-8" in:fade={{ duration: motionDuration('base') }}>
			<header class="flex flex-wrap items-start justify-between gap-4">
				<div class="flex min-w-0 flex-col gap-2">
					<h1 class="text-2xl font-semibold break-words text-fg">{application.company}</h1>
					<div class="flex flex-wrap items-center gap-2">
						<p class="text-base text-fg-muted">{application.title}</p>
						<StatusBadge status={application.status} />
					</div>
				</div>
				<div class="flex gap-2">
					<Button variant="secondary" onclick={() => (editing = true)}>Edit</Button>
					<Button variant="danger" onclick={() => (confirmingDelete = true)}>Delete</Button>
				</div>
			</header>

			<Card>
				<div class="flex flex-col gap-6">
					<div class="sm:w-60">
						<StatusSelect
							bind:value={status}
							disabled={savingStatus}
							onchange={(event) => {
								const next = event.currentTarget.value;
								if (isStatus(next)) void changeStatus(application, next);
							}}
						/>
					</div>

					<dl class="grid gap-x-8 gap-y-4 text-sm sm:grid-cols-2">
						{#snippet item(label: string, value: string | null)}
							<div class="flex flex-col gap-1">
								<dt class="text-fg-muted">{label}</dt>
								<dd class="break-words text-fg">{value ?? '—'}</dd>
							</div>
						{/snippet}
						{@render item('Location', application.location)}
						{@render item('Compensation', application.comp)}
						{@render item('Source', application.source)}
						<div class="flex flex-col gap-1">
							<dt class="text-fg-muted">Job posting</dt>
							<dd class="break-all text-fg">
								{#if application.url && isWebUrl(application.url)}
									<Link href={application.url} external>{application.url}</Link>
								{:else}
									{application.url ?? '—'}
								{/if}
							</dd>
						</div>
						{@render item('Added', formatDate(application.created_at))}
						{@render item('Updated', formatDate(application.updated_at))}
						{@render item('Status changed', formatDate(application.status_changed_at))}
					</dl>

					<section
						class="flex flex-col gap-2 border-t border-border pt-4"
						aria-labelledby="resume-heading"
					>
						<h2 id="resume-heading" class="text-sm font-semibold text-fg">Resume</h2>
						{#if resumes.kind === 'loading'}
							<Skeleton class="h-10 sm:w-60" />
						{:else if resumes.kind === 'error'}
							<p class="text-sm text-fg-subtle" in:fade={{ duration: motionDuration('base') }}>
								Couldn't load resumes
							</p>
						{:else if resumes.versions.length === 0}
							<p class="text-sm text-fg-subtle" in:fade={{ duration: motionDuration('base') }}>
								No resumes uploaded yet. <Link href={resumesHref}>Upload one</Link>
							</p>
						{:else}
							{@const linked = resumes.versions.find(
								(v) => v.id === application.resume_version_id
							)}
							<div
								class="flex flex-wrap items-end gap-x-4 gap-y-2"
								in:fade={{ duration: motionDuration('base') }}
							>
								<div class="w-full sm:w-60">
									<ResumeSelect
										versions={resumes.versions}
										bind:value={resumeId}
										disabled={savingResume}
										onchange={(event) => {
											const next = event.currentTarget.value;
											void changeResume(application, next === '' ? null : Number(next));
										}}
									/>
								</div>
								{#if linked}
									<p class="flex h-10 items-center text-sm">
										<Link href={resolve('/resumes/[id]', { id: String(linked.id) })}>
											{linked.name}
										</Link>
									</p>
								{/if}
							</div>
						{/if}
					</section>

					<section
						class="flex flex-col gap-2 border-t border-border pt-4"
						aria-labelledby="notes-heading"
					>
						<h2 id="notes-heading" class="text-sm font-semibold text-fg">Notes</h2>
						{#if application.notes}
							<p class="text-sm break-words whitespace-pre-wrap text-fg">{application.notes}</p>
						{:else}
							<p class="text-sm text-fg-subtle">No notes</p>
						{/if}
					</section>
				</div>
			</Card>
		</div>

		<ApplicationFormDialog
			bind:open={editing}
			mode="edit"
			initial={application}
			save={(values) => updateApplication(client, application.id, values)}
			onsaved={show}
		/>

		<Dialog
			bind:open={confirmingDelete}
			title="Delete application?"
			description="{application.company} · {application.title} will be removed permanently."
			size="sm"
		>
			{#snippet footer()}
				<Button variant="secondary" onclick={() => (confirmingDelete = false)}>Cancel</Button>
				<Button variant="danger" disabled={deleting} onclick={() => remove(application)}>
					{deleting ? 'Deleting…' : 'Delete'}
				</Button>
			{/snippet}
		</Dialog>
	{/if}
</div>
