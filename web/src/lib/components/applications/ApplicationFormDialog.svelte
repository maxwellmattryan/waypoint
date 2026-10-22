<script lang="ts" module>
	import type {
		Application,
		ApplicationInput,
		FieldError,
		NewApplication
	} from '#lib/api';
	import type { Result } from '#lib/applications/api.js';

	export type ApplicationFormDialogProps = {
		/** Bindable. */
		open?: boolean;
		/** Prefilled values; the current application when editing. */
		initial?: Partial<NewApplication>;
		/** Optional text under the title, e.g. where prefilled values came from. */
		description?: string;
		/** Called with the stored application after a successful save; the dialog is closed by then. */
		onsaved?: (application: Application) => void;
	} & (
		| { mode: 'create'; save: (values: NewApplication) => Promise<Result<Application>> }
		| { mode: 'edit'; save: (values: ApplicationInput) => Promise<Result<Application>> }
	);
</script>

<script lang="ts">
	import { Button, Dialog, toast } from '#lib/components/ui';
	import ApplicationForm, { FIELDS } from './ApplicationForm.svelte';

	let { open = $bindable(false), ...props }: ApplicationFormDialogProps = $props();

	const formId = $props.id();
	let submitting = $state(false);
	let errors = $state<FieldError[]>([]);

	const title = $derived(props.mode === 'create' ? 'Add application' : 'Edit application');
	const submitLabel = $derived(props.mode === 'create' ? 'Add application' : 'Save');

	const known = new Set<string>(FIELDS);

	function close() {
		open = false;
		errors = [];
	}

	async function submit(save: () => Promise<Result<Application>>) {
		submitting = true;
		errors = [];
		const result = await save();
		submitting = false;
		switch (result.kind) {
			case 'ok':
				close();
				toast.success(props.mode === 'create' ? 'Application added' : 'Application saved');
				props.onsaved?.(result.data);
				break;
			case 'invalid': {
				errors = result.errors.filter((e) => known.has(e.field));
				const other = result.errors.filter((e) => !known.has(e.field));
				if (other.length > 0) {
					toast.error("Couldn't save the application", {
						description: other.map((e) => e.message).join(' ')
					});
				}
				break;
			}
			default:
				toast.error("Couldn't save the application");
		}
	}
</script>

<Dialog
	bind:open
	{title}
	description={props.description}
	size="lg"
	onclose={() => (errors = [])}
>
	{#if props.mode === 'create'}
		{@const save = props.save}
		<ApplicationForm
			id={formId}
			mode="create"
			initial={props.initial}
			{submitting}
			{errors}
			onsubmit={(values) => submit(() => save(values))}
		/>
	{:else}
		{@const save = props.save}
		<ApplicationForm
			id={formId}
			mode="edit"
			initial={props.initial}
			{submitting}
			{errors}
			onsubmit={(values) => submit(() => save(values))}
		/>
	{/if}
	{#snippet footer()}
		<Button variant="secondary" onclick={close}>Cancel</Button>
		<Button type="submit" form={formId} disabled={submitting}>
			{submitting ? 'Saving…' : submitLabel}
		</Button>
	{/snippet}
</Dialog>
