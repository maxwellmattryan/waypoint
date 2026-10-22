<script lang="ts" module>
	import type { FieldError, ResumeVersion } from '#lib/api';
	import type { Result } from '#lib/resumes/api.js';

	export interface ResumeUploadDialogProps {
		/** Bindable. */
		open?: boolean;
		/** Sends the upload; called only once the client checks pass. */
		save: (file: File, name: string) => Promise<Result<ResumeVersion>>;
		/** Called with the stored version after a successful upload; the dialog is closed by then. */
		onsaved?: (version: ResumeVersion) => void;
	}
</script>

<script lang="ts">
	import { Button, Dialog, FileInput, Input, toast } from '#lib/components/ui';
	import { MAX_PDF_BYTES, TOO_LARGE, defaultName, isPdfFile } from '#lib/resumes/format.js';

	let { open = $bindable(false), save, onsaved }: ResumeUploadDialogProps = $props();

	const formId = $props.id();
	let file = $state<File | null>(null);
	let name = $state('');
	/** Whether the user typed the name, so picking a file no longer replaces it. */
	let nameTyped = $state(false);
	let submitting = $state(false);
	let errors = $state<{ file?: string; name?: string }>({});

	function reset() {
		file = null;
		name = '';
		nameTyped = false;
		errors = {};
	}

	function close() {
		open = false;
		reset();
	}

	function picked(next: File | null) {
		file = next;
		errors = { ...errors, file: undefined };
		if (next && (!nameTyped || name.trim() === '')) name = defaultName(next.name);
	}

	function check(): { file?: string; name?: string } {
		const found: { file?: string; name?: string } = {};
		if (!file) found.file = 'Choose a PDF';
		else if (file.size > MAX_PDF_BYTES) found.file = TOO_LARGE;
		else if (!isPdfFile(file)) found.file = 'Must be a PDF';
		if (name.trim() === '') found.name = 'Required';
		return found;
	}

	/** Puts server errors on their fields: `file` and `filename` on the file field. */
	function place(serverErrors: FieldError[]) {
		const placed: { file?: string; name?: string } = {};
		const other: string[] = [];
		for (const error of serverErrors) {
			if (error.field === 'name') placed.name ??= error.message;
			else if (error.field === 'file' || error.field === 'filename') placed.file ??= error.message;
			else other.push(error.message);
		}
		errors = placed;
		if (other.length > 0) {
			toast.error("Couldn't upload the resume", { description: other.join(' ') });
		}
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (submitting) return;
		const found = check();
		errors = found;
		if (found.file || found.name || !file) return;

		submitting = true;
		const result = await save(file, name.trim());
		submitting = false;
		switch (result.kind) {
			case 'ok':
				close();
				toast.success('Resume uploaded');
				onsaved?.(result.data);
				break;
			case 'invalid':
				place(result.errors);
				break;
			default:
				toast.error("Couldn't upload the resume");
		}
	}
</script>

<Dialog bind:open title="Upload resume" onclose={reset}>
	<form id={formId} class="flex flex-col gap-4" novalidate onsubmit={submit}>
		<FileInput
			label="PDF file"
			accept="application/pdf,.pdf"
			buttonLabel="Choose PDF"
			error={errors.file}
			disabled={submitting}
			onchange={picked}
		/>
		<Input
			label="Name"
			hint="A label for this version, e.g. Backend, Oct 2026."
			maxlength={200}
			error={errors.name}
			disabled={submitting}
			bind:value={name}
			oninput={() => (nameTyped = true)}
		/>
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={close}>Cancel</Button>
		<Button type="submit" form={formId} disabled={submitting}>
			{submitting ? 'Uploading…' : 'Upload'}
		</Button>
	{/snippet}
</Dialog>
