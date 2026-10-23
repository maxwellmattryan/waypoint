<script lang="ts" module>
	import type { GoalDetail, Plan, PlanEdit } from '#lib/api';
	import type { GoalResult } from '#lib/goals/api.js';

	export interface PlanEditorProps {
		/** Bindable. */
		open?: boolean;
		/** The current plan; the editor starts from it and sends its id as `base_plan_id`. */
		plan: Plan;
		/** Sends the edited plan. */
		onsave: (body: PlanEdit) => Promise<GoalResult<GoalDetail>>;
	}
</script>

<script lang="ts">
	import { flip } from 'svelte/animate';
	import { fade } from 'svelte/transition';
	import {
		Button,
		Checkbox,
		Drawer,
		Input,
		Select,
		TagInput,
		Textarea,
		type SelectOption
	} from '#lib/components/ui';
	import {
		companyErrors,
		draftErrors,
		draftOf,
		editBody,
		type CompanyDraft,
		type PlanDraft
	} from '#lib/goals/format.js';
	import { STALE_PLAN } from '#lib/goals/goal.svelte.js';
	import { easings, motionDuration } from '#lib/motion.js';
	import { SOURCE_LABELS, SOURCES } from '#lib/postings/api.js';
	import CompanyRow from './CompanyRow.svelte';

	/** The plan editor: companies, filters, shortlist settings and prep notes, saved as a new version. */
	let { open = $bindable(false), plan, onsave }: PlanEditorProps = $props();

	const NETWORK = "Couldn't save the plan. Check that the API is running.";
	const MAX_COMPANIES = 20;
	const options: SelectOption[] = SOURCES.map((value) => ({ value, label: SOURCE_LABELS[value] }));
	const blank = (): CompanyDraft => ({ name: '', source: '', board: '', reason: '' });
	const formId = $props.id();

	let base = $state(0);
	// Filled from the plan on every open.
	let draft = $state<PlanDraft>({
		companies: [],
		title_keywords: [],
		locations: [],
		remote_only: false,
		max_to_score: '',
		shortlist_size: '',
		min_score: '',
		prep_notes: ''
	});
	let company = $state<CompanyDraft>(blank());
	/** The index of the company being edited, or null when the form adds one. */
	let editing = $state<number | null>(null);
	let companyFieldErrors = $state<Partial<Record<'name' | 'source' | 'board', string>>>({});
	let errors = $state<Record<string, string>>({});
	let failure = $state<string | null>(null);
	let stale = $state(false);
	let busy = $state(false);

	function start(from: Plan) {
		base = from.id;
		draft = draftOf(from);
		company = blank();
		editing = null;
		companyFieldErrors = {};
		errors = {};
	}

	// A fresh copy of the current plan on every open.
	$effect(() => {
		if (open) {
			start(plan);
			failure = null;
			stale = false;
		}
	});

	// After a 409 the goal reloads; continue from the newer plan and keep the message.
	$effect(() => {
		if (open && stale && plan.id !== base) {
			start(plan);
			stale = false;
		}
	});

	const others = $derived(draft.companies.filter((_, index) => index !== editing));

	function saveCompany() {
		companyFieldErrors = companyErrors(company, others);
		if (Object.keys(companyFieldErrors).length > 0) return;
		const next = { ...company, name: company.name.trim(), board: company.board.trim() };
		if (editing === null) draft.companies = [...draft.companies, next];
		else draft.companies = draft.companies.map((c, index) => (index === editing ? next : c));
		company = blank();
		editing = null;
		errors = {};
	}

	function editCompany(index: number) {
		editing = index;
		company = { ...draft.companies[index] };
		companyFieldErrors = {};
	}

	function removeCompany(index: number) {
		draft.companies = draft.companies.filter((_, at) => at !== index);
		if (editing === index) {
			editing = null;
			company = blank();
		}
		errors = {};
	}

	/** The errors of company row `index`, as one line. */
	function rowErrors(index: number): string | null {
		const found = ['name', 'source', 'board']
			.map((field) => errors[`companies[${index}].${field}`])
			.filter((message) => message !== undefined);
		return found.length ? found.join('; ') : null;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		failure = null;
		errors = draftErrors(draft);
		if (Object.keys(errors).length > 0) return;
		busy = true;
		const result = await onsave(editBody(base, draft));
		busy = false;
		switch (result.kind) {
			case 'ok':
				open = false;
				return;
			case 'conflict':
				failure = STALE_PLAN;
				stale = true;
				return;
			case 'invalid': {
				const mapped: Record<string, string> = {};
				for (const error of result.errors) mapped[error.field] ??= error.message;
				errors = mapped;
				if (Object.keys(mapped).length === 0) failure = NETWORK;
				return;
			}
			default:
				failure = NETWORK;
		}
	}
</script>

<Drawer
	bind:open
	title="Edit plan"
	description="Saving writes a new version. Steps whose inputs don't change keep their results."
>
	<form id={formId} class="flex flex-col gap-8" aria-label="Edit plan" onsubmit={save} novalidate>
		<fieldset class="flex flex-col gap-4">
			<legend class="mb-2 text-sm font-semibold text-fg">Target companies</legend>
			{#if draft.companies.length > 0}
				<ul class="flex flex-col divide-y divide-border" aria-label="Companies">
					{#each draft.companies as row, index (`${row.source}/${row.board}`)}
						<li
							animate:flip={{ duration: motionDuration('base'), easing: easings.standard }}
							in:fade={{ duration: motionDuration('fast') }}
						>
							<CompanyRow company={row}>
								{#snippet actions()}
									<Button
										variant="ghost"
										size="sm"
										aria-label="Edit {row.name}"
										disabled={busy}
										onclick={() => editCompany(index)}
									>
										Edit
									</Button>
									<Button
										variant="ghost"
										size="sm"
										aria-label="Remove {row.name}"
										disabled={busy}
										onclick={() => removeCompany(index)}
									>
										Remove
									</Button>
								{/snippet}
							</CompanyRow>
							{#if rowErrors(index)}
								<p class="pb-2 text-sm text-danger">{rowErrors(index)}</p>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
			{#if errors.companies}
				<p class="text-sm text-danger" role="alert">{errors.companies}</p>
			{/if}
			<div class="flex flex-col gap-4 rounded-md border border-border p-4">
				<p class="text-sm font-semibold text-fg">
					{editing === null ? 'Add a company' : `Edit ${draft.companies[editing]?.name ?? ''}`}
				</p>
				<Input label="Company name" bind:value={company.name} disabled={busy} error={companyFieldErrors.name} />
				<div class="grid gap-4 sm:grid-cols-2">
					<Select
						label="Board"
						placeholder="Choose a board"
						{options}
						bind:value={company.source}
						disabled={busy}
						error={companyFieldErrors.source}
					/>
					<Input
						label="Board slug"
						autocomplete="off"
						spellcheck="false"
						bind:value={company.board}
						disabled={busy}
						error={companyFieldErrors.board}
					/>
				</div>
				<Input label="Reason (optional)" bind:value={company.reason} disabled={busy} />
				<div class="flex justify-end gap-2">
					{#if editing !== null}
						<Button
							variant="ghost"
							size="sm"
							disabled={busy}
							onclick={() => {
								editing = null;
								company = blank();
								companyFieldErrors = {};
							}}
						>
							Cancel edit
						</Button>
					{/if}
					<Button
						variant="secondary"
						size="sm"
						disabled={busy || (editing === null && draft.companies.length >= MAX_COMPANIES)}
						onclick={saveCompany}
					>
						{editing === null ? 'Add company' : 'Update company'}
					</Button>
				</div>
			</div>
		</fieldset>

		<fieldset class="flex flex-col gap-4">
			<legend class="mb-2 text-sm font-semibold text-fg">Filters</legend>
			<TagInput
				label="Title keywords"
				hint="A posting's title must contain one. Press Enter after each."
				bind:value={draft.title_keywords}
				max={10}
				disabled={busy}
				error={errors.title_keywords ??
					Object.entries(errors).find(([field]) => field.startsWith('title_keywords['))?.[1]}
			/>
			<TagInput
				label="Locations"
				hint="Leave empty for anywhere."
				bind:value={draft.locations}
				max={10}
				disabled={busy}
				error={errors.locations ??
					Object.entries(errors).find(([field]) => field.startsWith('locations['))?.[1]}
			/>
			<Checkbox label="Remote only" bind:checked={draft.remote_only} disabled={busy} />
			<Input
				label="Most postings to score"
				inputmode="numeric"
				hint="1 to 50"
				bind:value={draft.max_to_score}
				disabled={busy}
				error={errors.max_to_score}
			/>
		</fieldset>

		<fieldset class="grid gap-4 sm:grid-cols-2">
			<legend class="mb-2 text-sm font-semibold text-fg">Shortlist</legend>
			<Input
				label="Shortlist size"
				inputmode="numeric"
				hint="1 to 20"
				bind:value={draft.shortlist_size}
				disabled={busy}
				error={errors.shortlist_size}
			/>
			<Input
				label="Minimum score"
				inputmode="numeric"
				hint="0 to 100"
				bind:value={draft.min_score}
				disabled={busy}
				error={errors.min_score}
			/>
		</fieldset>

		<Textarea
			label="Prep notes"
			rows={3}
			bind:value={draft.prep_notes}
			disabled={busy}
			error={errors.prep_notes}
		/>

		{#if failure}
			<p class="text-sm text-danger" role="alert">{failure}</p>
		{/if}
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={() => (open = false)}>Cancel</Button>
		<Button type="submit" form={formId} disabled={busy}>{busy ? 'Saving…' : 'Save plan'}</Button>
	{/snippet}
</Drawer>
