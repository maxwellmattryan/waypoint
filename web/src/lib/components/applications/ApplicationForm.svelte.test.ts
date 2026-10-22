import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Application } from '#lib/api';
import type { Result } from '#lib/applications/api.js';
import { toast } from '#lib/components/ui';
import ApplicationForm from './ApplicationForm.svelte';
import ApplicationFormDialog from './ApplicationFormDialog.svelte';

afterEach(() => toast.clear());

const application: Application = {
	id: 3,
	company: 'Acme',
	title: 'Engineer',
	url: null,
	location: 'Remote',
	comp: null,
	source: null,
	notes: null,
	status: 'sourced',
	status_changed_at: '2026-10-22T00:00:00Z',
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:00Z'
};

function submit(form: HTMLElement) {
	form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

describe('ApplicationForm', () => {
	it('requires company and title and sends nothing while they are blank', async () => {
		const user = userEvent.setup();
		const onsubmit = vi.fn();
		render(ApplicationForm, { mode: 'create', onsubmit });

		submit(screen.getByRole('form'));
		await waitFor(() => expect(screen.getAllByText('Required')).toHaveLength(2));
		expect(screen.getByLabelText('Company')).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByLabelText('Title')).toHaveAttribute('aria-invalid', 'true');

		await user.type(screen.getByLabelText('Company'), '   ');
		await user.type(screen.getByLabelText('Title'), 'Engineer');
		submit(screen.getByRole('form'));
		await waitFor(() => expect(screen.getAllByText('Required')).toHaveLength(1));
		expect(onsubmit).not.toHaveBeenCalled();
	});

	it('passes trimmed values, blank optionals as null, and the chosen status in create mode', async () => {
		const user = userEvent.setup();
		const onsubmit = vi.fn();
		render(ApplicationForm, { mode: 'create', onsubmit });

		const status = screen.getByLabelText('Status');
		expect(status).toHaveValue('sourced');
		await user.type(screen.getByLabelText('Company'), '  Acme  ');
		await user.type(screen.getByLabelText('Title'), ' Engineer ');
		await user.type(screen.getByLabelText('Location'), '   ');
		await user.type(screen.getByLabelText('Notes'), ' line one{Enter}line two ');
		await user.selectOptions(status, 'applied');
		submit(screen.getByRole('form'));

		expect(onsubmit).toHaveBeenCalledExactlyOnceWith({
			company: 'Acme',
			title: 'Engineer',
			url: null,
			location: null,
			comp: null,
			source: null,
			notes: 'line one\nline two',
			status: 'applied'
		});
	});

	it('has no status field in edit mode and starts from the initial values', () => {
		const onsubmit = vi.fn();
		render(ApplicationForm, { mode: 'edit', initial: application, onsubmit });

		expect(screen.queryByLabelText('Status')).toBeNull();
		expect(screen.getByLabelText('Company')).toHaveValue('Acme');
		expect(screen.getByLabelText('Location')).toHaveValue('Remote');
		submit(screen.getByRole('form'));
		expect(onsubmit).toHaveBeenCalledExactlyOnceWith({
			company: 'Acme',
			title: 'Engineer',
			url: null,
			location: 'Remote',
			comp: null,
			source: null,
			notes: null
		});
	});

	it('caps every field at the server limit', () => {
		render(ApplicationForm, { mode: 'create', onsubmit: vi.fn() });
		const limits: [string, string][] = [
			['Company', '200'],
			['Title', '200'],
			['Job posting URL', '2048'],
			['Location', '200'],
			['Compensation', '200'],
			['Source', '200'],
			['Notes', '20000']
		];
		for (const [label, max] of limits) {
			expect(screen.getByLabelText(label)).toHaveAttribute('maxlength', max);
		}
	});

	it('shows server errors on their fields', () => {
		render(ApplicationForm, {
			mode: 'create',
			onsubmit: vi.fn(),
			errors: [
				{ field: 'url', message: 'Must be an http or https URL' },
				{ field: 'notes', message: 'At most 20000 characters' }
			]
		});
		expect(screen.getByLabelText('Job posting URL')).toHaveAccessibleDescription(
			'Must be an http or https URL'
		);
		expect(screen.getByLabelText('Notes')).toHaveAccessibleDescription('At most 20000 characters');
	});

	it('disables every field while submitting', () => {
		render(ApplicationForm, { mode: 'create', onsubmit: vi.fn(), submitting: true });
		for (const label of ['Company', 'Title', 'Job posting URL', 'Status', 'Notes']) {
			expect(screen.getByLabelText(label)).toBeDisabled();
		}
	});
});

describe('ApplicationFormDialog', () => {
	function deferred() {
		let settle!: (result: Result<Application>) => void;
		const promise = new Promise<Result<Application>>((resolve) => (settle = resolve));
		return { promise, settle };
	}

	async function fillAndSubmit(user: ReturnType<typeof userEvent.setup>) {
		await user.type(screen.getByLabelText('Company'), 'Acme');
		await user.type(screen.getByLabelText('Title'), 'Engineer');
		await user.click(screen.getByRole('button', { name: 'Add application' }));
	}

	it('reads Saving… and disables the submit button while saving, then closes and toasts', async () => {
		const user = userEvent.setup();
		const pending = deferred();
		const save = vi.fn(() => pending.promise);
		const onsaved = vi.fn();
		render(ApplicationFormDialog, { open: true, mode: 'create', save, onsaved });

		expect(screen.getByLabelText('Company')).toHaveFocus();
		await fillAndSubmit(user);
		const saving = await screen.findByRole('button', { name: 'Saving…' });
		expect(saving).toBeDisabled();
		expect(screen.getByLabelText('Company')).toBeDisabled();

		pending.settle({ kind: 'ok', data: application });
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(onsaved).toHaveBeenCalledExactlyOnceWith(application);
		expect(toast.items.map((t) => [t.tone, t.title])).toEqual([['success', 'Application added']]);
	});

	it('keeps the dialog open with the input on a 422 and shows each error on its field', async () => {
		const user = userEvent.setup();
		const save = vi.fn(
			async (): Promise<Result<Application>> => ({
				kind: 'invalid',
				errors: [
					{ field: 'company', message: 'At most 200 characters' },
					{ field: 'mystery', message: 'Something else' }
				]
			})
		);
		render(ApplicationFormDialog, { open: true, mode: 'create', save });

		await fillAndSubmit(user);
		await waitFor(() =>
			expect(screen.getByLabelText('Company')).toHaveAccessibleDescription('At most 200 characters')
		);
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByLabelText('Company')).toHaveValue('Acme');
		expect(toast.items.map((t) => [t.tone, t.title])).toEqual([
			['danger', "Couldn't save the application"]
		]);
	});

	it('keeps the dialog open with the input on any other failure', async () => {
		const user = userEvent.setup();
		const save = vi.fn(async (): Promise<Result<Application>> => ({ kind: 'failed' }));
		render(ApplicationFormDialog, { open: true, mode: 'create', save });

		await fillAndSubmit(user);
		await waitFor(() => expect(toast.items).toHaveLength(1));
		expect(toast.items[0]).toMatchObject({ tone: 'danger', title: "Couldn't save the application" });
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByLabelText('Title')).toHaveValue('Engineer');
		expect(screen.getByRole('button', { name: 'Add application' })).toBeEnabled();
	});
});
