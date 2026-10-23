<script lang="ts" module>
	import type { FieldError, ProfileImportSummary } from '#lib/api';
	import type { Result } from '#lib/memories/profile.js';

	export interface LinkedInImportDialogProps {
		/** Bindable. */
		open?: boolean;
		/** Sends the export; called only once the client checks pass. */
		save: (file: File) => Promise<Result<ProfileImportSummary>>;
		/** Called with the summary after a successful import; the dialog is closed by then. */
		onimported?: (summary: ProfileImportSummary) => void;
	}
</script>

<script lang="ts">
	import { Button, Dialog, FileInput, toast } from '#lib/components/ui';
	import { MAX_ZIP_BYTES, ZIP_TOO_LARGE, summaryText } from '#lib/memories/profile.js';

	let { open = $bindable(false), save, onimported }: LinkedInImportDialogProps = $props();

	const formId = $props.id();
	const FAILED = "Couldn't import the LinkedIn export";
	let file = $state<File | null>(null);
	let submitting = $state(false);
	let error = $state<string | undefined>();

	function reset() {
		file = null;
		error = undefined;
	}

	function close() {
		open = false;
		reset();
	}

	function picked(next: File | null) {
		file = next;
		error = undefined;
	}

	function check(): string | undefined {
		if (!file) return 'Choose a .zip file';
		if (file.size > MAX_ZIP_BYTES) return ZIP_TOO_LARGE;
		if (!/\.zip$/i.test(file.name)) return 'Must be a .zip file';
		return undefined;
	}

	/** Puts `file` and `filename` errors on the file field and toasts any other. */
	function place(serverErrors: FieldError[]) {
		const isFileField = (e: FieldError) => e.field === 'file' || e.field === 'filename';
		error = serverErrors.find(isFileField)?.message;
		const other = serverErrors.filter((e) => !isFileField(e));
		if (!error || other.length > 0) {
			toast.error(FAILED, { description: other.map((e) => e.message).join(' ') || undefined });
		}
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (submitting) return;
		error = check();
		if (error || !file) return;

		submitting = true;
		const result = await save(file);
		submitting = false;
		switch (result.kind) {
			case 'ok':
				close();
				toast.success('LinkedIn export imported', { description: summaryText(result.data) });
				onimported?.(result.data);
				break;
			case 'invalid':
				place(result.errors);
				break;
			default:
				toast.error(FAILED);
		}
	}
</script>

<Dialog bind:open title="Import LinkedIn export" onclose={reset}>
	<form id={formId} class="flex flex-col gap-4" novalidate onsubmit={submit}>
		<FileInput
			label="LinkedIn export"
			accept=".zip,application/zip"
			buttonLabel="Choose .zip"
			hint="From LinkedIn: Settings → Data privacy → Get a copy of your data. Waypoint reads Positions, Skills and Education."
			{error}
			disabled={submitting}
			onchange={picked}
		/>
	</form>
	{#snippet footer()}
		<Button variant="secondary" onclick={close}>Cancel</Button>
		<Button type="submit" form={formId} disabled={submitting}>
			{submitting ? 'Importing…' : 'Import'}
		</Button>
	{/snippet}
</Dialog>
