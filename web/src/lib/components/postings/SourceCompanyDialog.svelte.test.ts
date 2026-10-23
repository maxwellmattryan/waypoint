import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { createApiClient, type Job } from '#lib/api';
import SourceCompanyDialog from './SourceCompanyDialog.svelte';

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

const job: Job = {
	id: 11,
	kind: 'source_board',
	status: 'queued',
	attempts: 0,
	max_attempts: 3,
	run_at: '2026-10-22T00:00:00Z',
	last_error: null,
	result: null,
	created_at: '2026-10-22T00:00:00Z',
	updated_at: '2026-10-22T00:00:00Z',
	finished_at: null
};

/** Renders the open dialog with a `fetch` that answers `POST /sourcing/boards` with `answer()`. */
function setup(answer: () => Response | Promise<Response>) {
	const fetch = vi.fn<typeof globalThis.fetch>(async () => answer());
	const onqueued = vi.fn();
	const view = render(SourceCompanyDialog, {
		open: true,
		client: createApiClient({ baseUrl: 'http://api.test', fetch }),
		onqueued
	});
	return { fetch, onqueued, view, user: userEvent.setup() };
}

const board = () => screen.getByLabelText('Board');
const slug = () => screen.getByLabelText('Board slug');
const submit = () => screen.getByRole('button', { name: 'Source' });

async function fill(user: ReturnType<typeof userEvent.setup>, source: string, text: string) {
	if (source) await user.selectOptions(board(), source);
	if (text) await user.type(slug(), text);
	await user.click(submit());
}

describe('SourceCompanyDialog', () => {
	it.each([
		['', '', 'Required', 'Required'],
		['lever', '   ', null, 'Required'],
		['', 'example-labs', 'Required', null],
		['ashby', 'example labs', null, 'Must be 1–100 letters, digits, hyphens or underscores'],
		['greenhouse', 'a'.repeat(101), null, 'Must be 1–100 letters, digits, hyphens or underscores']
	])(
		'blocks board %o with slug %o and sends nothing',
		async (source, text, boardError, slugError) => {
			const { fetch, user } = setup(() => json({ job, reused: false }, 202));
			await fill(user, source, text);
			if (boardError) expect(board()).toHaveAccessibleDescription(boardError);
			else expect(board()).not.toHaveAttribute('aria-invalid');
			if (slugError) expect(slug()).toHaveAccessibleDescription(expect.stringContaining(slugError));
			else expect(slug()).not.toHaveAttribute('aria-invalid');
			expect(fetch).not.toHaveBeenCalled();
		}
	);

	it('posts the trimmed slug, reads "Queuing…" while in flight, and closes on 202', async () => {
		let answer!: (response: Response) => void;
		const { fetch, onqueued, user } = setup(() => new Promise((r) => (answer = r)));
		await fill(user, 'greenhouse', '  Example-Labs_2 ');
		const queuing = await screen.findByRole('button', { name: 'Queuing…' });
		expect(queuing).toBeDisabled();
		const request = fetch.mock.calls[0][0] as Request;
		expect(request.method).toBe('POST');
		expect(new URL(request.url).pathname).toBe('/sourcing/boards');
		expect(await request.clone().json()).toEqual({ source: 'greenhouse', board: 'Example-Labs_2' });

		answer(json({ job, reused: true }, 202));
		await waitFor(() =>
			expect(onqueued).toHaveBeenCalledWith({ jobId: 11, slug: 'Example-Labs_2', reused: true })
		);
		await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
	});

	it('maps 422 field errors onto the fields and stays open', async () => {
		const { onqueued, user } = setup(() =>
			json(
				{
					errors: [
						{ field: 'source', message: 'Must be greenhouse, lever or ashby' },
						{ field: 'board', message: 'Taken elsewhere' }
					]
				},
				422
			)
		);
		await fill(user, 'lever', 'example-labs');
		await waitFor(() =>
			expect(board()).toHaveAccessibleDescription('Must be greenhouse, lever or ashby')
		);
		expect(slug()).toHaveAccessibleDescription(expect.stringContaining('Taken elsewhere'));
		expect(screen.getByRole('dialog', { name: 'Source a company' })).toBeInTheDocument();
		expect(onqueued).not.toHaveBeenCalled();
	});

	it.each([
		['a network error', () => Promise.reject(new TypeError('offline'))],
		['a 500', () => json(null, 500)]
	])('shows the error inline on %s and stays open', async (_, answer) => {
		const { onqueued, user } = setup(answer);
		await fill(user, 'ashby', 'example-labs');
		expect(await screen.findByRole('alert')).toHaveTextContent(
			"Couldn't queue the fetch. Check that the API is running."
		);
		expect(submit()).toBeEnabled();
		expect(onqueued).not.toHaveBeenCalled();
	});

	it('resets every time it opens', async () => {
		const { view, user } = setup(() => json(null, 500));
		await fill(user, '', 'bad slug');
		expect(slug()).toHaveAttribute('aria-invalid', 'true');
		await view.rerender({ open: false });
		await view.rerender({ open: true });
		expect(slug()).toHaveValue('');
		expect(board()).toHaveValue('');
		expect(slug()).not.toHaveAttribute('aria-invalid');
	});
});
