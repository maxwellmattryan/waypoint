import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ResumeVersion } from '#lib/api';
import type { Result } from '#lib/resumes/api.js';
import { toast } from '#lib/components/ui';
import { MAX_PDF_BYTES } from '#lib/resumes/format.js';
import ResumeUploadDialog from './ResumeUploadDialog.svelte';

afterEach(() => toast.clear());

const version: ResumeVersion = {
	id: 7,
	name: 'Backend CV',
	filename: 'Backend CV.PDF',
	size_bytes: 13,
	sha256: 'ab'.repeat(32),
	has_text: true,
	application_count: 0,
	created_at: '2026-10-22T00:00:00Z'
};

const pdf = (name = 'Backend CV.PDF', type = 'application/pdf') =>
	new File(['%PDF-1.4 fake'], name, { type });

function setup(save: (file: File, name: string) => Promise<Result<ResumeVersion>>) {
	// `applyAccept: false` lets a test pick a file the `accept` filter would hide.
	const user = userEvent.setup({ applyAccept: false });
	const onsaved = vi.fn();
	render(ResumeUploadDialog, { open: true, save, onsaved });
	return {
		user,
		onsaved,
		file: () => screen.getByLabelText('PDF file'),
		name: () => screen.getByLabelText('Name'),
		submit: () => user.click(screen.getByRole('button', { name: 'Upload' }))
	};
}

describe('ResumeUploadDialog', () => {
	it('accepts only PDFs in the picker', () => {
		setup(vi.fn());
		expect(screen.getByLabelText('PDF file')).toHaveAttribute('accept', 'application/pdf,.pdf');
	});

	it('names the version after the picked file unless the user typed a name', async () => {
		const { user, file, name } = setup(vi.fn());
		await user.upload(file(), pdf());
		expect(name()).toHaveValue('Backend CV');
		await user.upload(file(), pdf('Frontend.pdf'));
		expect(name()).toHaveValue('Frontend');

		await user.clear(name());
		await user.type(name(), 'My label');
		await user.upload(file(), pdf('Other.pdf'));
		expect(name()).toHaveValue('My label');
	});

	it('refuses a missing file, a non-PDF, a file over 10 MiB and a blank name without saving', async () => {
		const save = vi.fn();
		const { user, file, name, submit } = setup(save);

		await submit();
		await waitFor(() => expect(file()).toHaveAccessibleDescription('Choose a PDF'));
		expect(name()).toHaveAccessibleDescription(/Required/);

		await user.upload(file(), pdf('cv.docx', 'application/msword'));
		await submit();
		await waitFor(() => expect(file()).toHaveAccessibleDescription('Must be a PDF'));

		const huge = pdf('huge.pdf');
		Object.defineProperty(huge, 'size', { value: MAX_PDF_BYTES + 1 });
		await user.upload(file(), huge);
		await submit();
		await waitFor(() => expect(file()).toHaveAccessibleDescription('At most 10 MB'));

		await user.upload(file(), pdf());
		await user.clear(name());
		await user.type(name(), '   ');
		await submit();
		await waitFor(() => expect(name()).toHaveAccessibleDescription(/Required/));
		expect(file()).not.toHaveAccessibleDescription('At most 10 MB');

		expect(save).not.toHaveBeenCalled();
	});

	it('reads Uploading… while saving, then closes, toasts and hands back the version', async () => {
		let settle!: (result: Result<ResumeVersion>) => void;
		const save = vi.fn(() => new Promise<Result<ResumeVersion>>((resolve) => (settle = resolve)));
		const { user, file, submit, onsaved } = setup(save);

		const picked = pdf();
		await user.upload(file(), picked);
		await submit();
		const uploading = await screen.findByRole('button', { name: 'Uploading…' });
		expect(uploading).toBeDisabled();
		expect(save).toHaveBeenCalledExactlyOnceWith(picked, 'Backend CV');

		settle({ kind: 'ok', data: version });
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(onsaved).toHaveBeenCalledExactlyOnceWith(version);
		expect(toast.items.map((t) => [t.tone, t.title])).toEqual([['success', 'Resume uploaded']]);
	});

	it('shows server errors on their fields and keeps the dialog open', async () => {
		const save = vi.fn(
			async (): Promise<Result<ResumeVersion>> => ({
				kind: 'invalid',
				errors: [
					{ field: 'name', message: 'At most 200 characters' },
					{ field: 'file', message: 'At most 10 MB' }
				]
			})
		);
		const { user, file, name, submit } = setup(save);
		await user.upload(file(), pdf());
		await submit();
		await waitFor(() => expect(file()).toHaveAccessibleDescription('At most 10 MB'));
		expect(name()).toHaveAccessibleDescription(/At most 200 characters/);
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(toast.items).toEqual([]);
	});

	it('toasts any other failure and keeps the dialog open', async () => {
		const save = vi.fn(async (): Promise<Result<ResumeVersion>> => ({ kind: 'failed' }));
		const { user, file, name, submit } = setup(save);
		await user.upload(file(), pdf());
		await submit();
		await waitFor(() => expect(toast.items).toHaveLength(1));
		expect(toast.items[0]).toMatchObject({ tone: 'danger', title: "Couldn't upload the resume" });
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(name()).toHaveValue('Backend CV');
		expect(screen.getByRole('button', { name: 'Upload' })).toBeEnabled();
	});
});
