<script lang="ts" module>
	import type { ApplicationInput, ApplicationStatus, FieldError, NewApplication } from '#lib/api';

	/** The API's limits, in characters after trimming. */
	export const LIMITS = {
		company: 200,
		title: 200,
		url: 2048,
		location: 200,
		comp: 200,
		source: 200,
		notes: 20000
	} as const;

	/** The text fields the form edits, in display order. */
	export type TextField = keyof typeof LIMITS;

	export const FIELDS = Object.keys(LIMITS) as TextField[];

	export type ApplicationFormProps = {
		/** `id` of the `<form>`, for a submit button outside it (`form={id}`). */
		id?: string;
		/** Prefilled values. */
		initial?: Partial<NewApplication>;
		/** Disables every field while a save is in flight. */
		submitting?: boolean;
		/** Server-side errors; each one shows on its field. */
		errors?: FieldError[];
	} & (
		| { mode: 'create'; onsubmit: (values: NewApplication) => void }
		| { mode: 'edit'; onsubmit: (values: ApplicationInput) => void }
	);
</script>

<script lang="ts">
	import { Input, Textarea } from '#lib/components/ui';
	import StatusSelect from './StatusSelect.svelte';

	const uid = $props.id();

	let props: ApplicationFormProps = $props();
	const id = $derived(props.id ?? uid);

	// The form owns its values from here on; `initial` only seeds them.
	// svelte-ignore state_referenced_locally
	const seed = props.initial ?? {};
	let values = $state<Record<TextField, string>>(
		Object.fromEntries(FIELDS.map((field) => [field, seed[field] ?? ''])) as Record<
			TextField,
			string
		>
	);
	let status = $state<ApplicationStatus>(seed.status ?? 'sourced');
	let required = $state<Partial<Record<TextField, string>>>({});

	const serverErrors = $derived(
		Object.fromEntries((props.errors ?? []).map((e) => [e.field, e.message])) as Partial<
			Record<string, string>
		>
	);

	function errorFor(field: TextField): string | undefined {
		return required[field] ?? serverErrors[field];
	}

	function onsubmit(event: SubmitEvent) {
		event.preventDefault();
		if (props.submitting) return;
		const trimmed = Object.fromEntries(
			FIELDS.map((field) => [field, values[field].trim()])
		) as Record<TextField, string>;
		const missing: Partial<Record<TextField, string>> = {};
		if (!trimmed.company) missing.company = 'Required';
		if (!trimmed.title) missing.title = 'Required';
		required = missing;
		if (Object.keys(missing).length > 0) return;

		const input: ApplicationInput = {
			company: trimmed.company,
			title: trimmed.title,
			url: trimmed.url || null,
			location: trimmed.location || null,
			comp: trimmed.comp || null,
			source: trimmed.source || null,
			notes: trimmed.notes || null
		};
		if (props.mode === 'create') props.onsubmit({ ...input, status });
		else props.onsubmit(input);
	}
</script>

<form {id} novalidate {onsubmit} aria-label={props.mode === 'create' ? 'New application' : 'Edit application'}>
	<fieldset disabled={props.submitting} class="flex min-w-0 flex-col gap-4">
		<div class="grid gap-4 sm:grid-cols-2">
			<Input
				label="Company"
				required
				maxlength={LIMITS.company}
				autocomplete="organization"
				data-autofocus
				bind:value={values.company}
				error={errorFor('company')}
			/>
			<Input
				label="Title"
				required
				maxlength={LIMITS.title}
				autocomplete="organization-title"
				bind:value={values.title}
				error={errorFor('title')}
			/>
		</div>
		<Input
			label="Job posting URL"
			type="url"
			inputmode="url"
			placeholder="https://"
			maxlength={LIMITS.url}
			bind:value={values.url}
			error={errorFor('url')}
		/>
		<div class="grid gap-4 sm:grid-cols-2">
			<Input
				label="Location"
				maxlength={LIMITS.location}
				bind:value={values.location}
				error={errorFor('location')}
			/>
			<Input
				label="Compensation"
				placeholder="$180k–$210k + equity"
				maxlength={LIMITS.comp}
				bind:value={values.comp}
				error={errorFor('comp')}
			/>
			<Input
				label="Source"
				placeholder="Referral"
				maxlength={LIMITS.source}
				bind:value={values.source}
				error={errorFor('source')}
			/>
			{#if props.mode === 'create'}
				<StatusSelect bind:value={status} />
			{/if}
		</div>
		<Textarea
			label="Notes"
			rows={6}
			maxlength={LIMITS.notes}
			bind:value={values.notes}
			error={errorFor('notes')}
		/>
	</fieldset>
</form>
