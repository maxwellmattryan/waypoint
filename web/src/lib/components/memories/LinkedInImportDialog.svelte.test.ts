import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProfileImportSummary } from '#lib/api';
import { toast } from '#lib/components/ui';
import { MAX_ZIP_BYTES, type Result } from '#lib/memories/profile.js';
import LinkedInImportDialog from './LinkedInImportDialog.svelte';

afterEach(() => toast.clear());

const summary: ProfileImportSummary = {
	added: 12,
	already_known: 3,
	skipped: 0
};

const zip = (name = 'Basic_LinkedInDataExport.ZIP', type = 'application/zip') =>
	new File(['PK fake'], name, { type });

function setup(save: (file: File) => Promise<Result<ProfileImportSummary>>) {
	// `applyAccept: false` lets a test pick a file the `accept` filter would hide.
	const user = userEvent.setup({ applyAccept: false });
	const onimported = vi.fn();
	render(LinkedInImportDialog, { open: true, save, onimported });
	return {
		user,
		onimported,
		file: () => screen.getByLabelText('LinkedIn export'),
		submit: () => user.click(screen.getByRole('button', { name: 'Import' }))
	};
}

describe('LinkedInImportDialog', () => {
	it('accepts only zips in the picker and says where the export comes from', () => {
		const { file } = setup(vi.fn());
		expect(file()).toHaveAttribute('accept', '.zip,application/zip');
		expect(file()).toHaveAccessibleDescription(
			'From LinkedIn: Settings → Data privacy → Get a copy of your data. Waypoint reads Positions, Skills and Education.'
		);
	});

	it('refuses a missing file, a non-zip and a file over 20 MiB without saving', async () => {
		const save = vi.fn();
		const { user, file, submit } = setup(save);

		await submit();
		await waitFor(() => expect(file()).toHaveAccessibleDescription(/Choose a \.zip file/));

		await user.upload(file(), zip('Positions.csv', 'text/csv'));
		await submit();
		await waitFor(() => expect(file()).toHaveAccessibleDescription(/Must be a \.zip file/));

		const huge = zip();
		Object.defineProperty(huge, 'size', { value: MAX_ZIP_BYTES + 1 });
		await user.upload(file(), huge);
		await submit();
		await waitFor(() => expect(file()).toHaveAccessibleDescription(/Must be 20 MiB or smaller/));

		expect(save).not.toHaveBeenCalled();
	});

	it('reads Importing… while saving, then closes, toasts the summary and reports it', async () => {
		let settle!: (result: Result<ProfileImportSummary>) => void;
		const save = vi.fn(
			() => new Promise<Result<ProfileImportSummary>>((resolve) => (settle = resolve))
		);
		const { user, file, submit, onimported } = setup(save);

		const picked = zip();
		await user.upload(file(), picked);
		await submit();
		const importing = await screen.findByRole('button', { name: 'Importing…' });
		expect(importing).toBeDisabled();
		expect(save).toHaveBeenCalledExactlyOnceWith(picked);

		settle({ kind: 'ok', data: summary });
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(onimported).toHaveBeenCalledExactlyOnceWith(summary);
		expect(toast.items.map((t) => [t.tone, t.title, t.description])).toEqual([
			['success', 'LinkedIn export imported', 'Added 12 memories, 3 already known']
		]);
	});

	it.each(['file', 'filename'])(
		'shows a server %s error on the field and keeps the dialog open',
		async (field) => {
			const save = vi.fn(async (): Promise<Result<ProfileImportSummary>> => ({
				kind: 'invalid',
				errors: [{ field, message: 'Skills.csv is too large' }]
			}));
			const { user, file, submit, onimported } = setup(save);
			await user.upload(file(), zip());
			await submit();
			await waitFor(() => expect(file()).toHaveAccessibleDescription(/Skills\.csv is too large/));
			expect(screen.getByRole('dialog')).toBeInTheDocument();
			expect(toast.items).toEqual([]);
			expect(onimported).not.toHaveBeenCalled();
		}
	);

	it('toasts any other failure and keeps the dialog open', async () => {
		const save = vi.fn(async (): Promise<Result<ProfileImportSummary>> => ({
			kind: 'failed'
		}));
		const { user, file, submit, onimported } = setup(save);
		await user.upload(file(), zip());
		await submit();
		await waitFor(() => expect(toast.items).toHaveLength(1));
		expect(toast.items[0]).toMatchObject({
			tone: 'danger',
			title: "Couldn't import the LinkedIn export"
		});
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Import' })).toBeEnabled();
		expect(onimported).not.toHaveBeenCalled();
	});
});
