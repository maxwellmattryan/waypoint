import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import {
	connectEvents,
	createApiClient,
	type Capture,
	type EventSourceLike,
	type Job
} from '#lib/api';
import CaptureDialog from './CaptureDialog.svelte';

/** A stand-in for the browser's EventSource that tests drive by hand. */
class FakeEventSource implements EventSourceLike {
	static last: FakeEventSource | null = null;
	onerror: ((event: globalThis.Event) => void) | null = null;
	closed = false;
	#listeners = new Map<string, ((event: MessageEvent) => void)[]>();

	constructor(readonly url: string) {
		FakeEventSource.last = this;
	}

	addEventListener(type: string, listener: (event: MessageEvent) => void): void {
		this.#listeners.set(type, [...(this.#listeners.get(type) ?? []), listener]);
	}

	close(): void {
		this.closed = true;
	}

	emit(type: string, data: object = {}): void {
		const message = new MessageEvent(type, {
			data: JSON.stringify({ type, ...data })
		});
		for (const listener of this.#listeners.get(type) ?? []) listener(message);
	}
}

const fakeEvents: typeof connectEvents = (handlers) =>
	connectEvents(handlers, {
		url: 'http://api.test/events',
		EventSource: FakeEventSource
	});

function json(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

const URL_TEXT = 'https://acme.example/jobs/1';

const capture: Capture = {
	method: 'llm',
	fields: {
		company: 'Acme',
		title: 'Engineer',
		url: URL_TEXT,
		location: null,
		comp: null
	}
};

function job(overrides: Partial<Job> = {}): Job {
	return {
		id: 7,
		kind: 'capture_extract',
		status: 'queued',
		attempts: 0,
		max_attempts: 3,
		run_at: '2026-10-22T00:00:00Z',
		last_error: null,
		result: null,
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		finished_at: null,
		...overrides
	};
}

/** Renders the open dialog with a `fetch` that answers `POST /captures` with `captures()`. */
function setup(
	captures: () => Response | Promise<Response>,
	jobs: () => Response = () => json(job())
) {
	const fetch = vi.fn<typeof globalThis.fetch>(async (input, init) => {
		const request = input instanceof Request ? input : new Request(input, init);
		return new URL(request.url).pathname === '/captures' ? captures() : jobs();
	});
	const oncaptured = vi.fn();
	const onmanual = vi.fn();
	render(CaptureDialog, {
		open: true,
		client: createApiClient({ baseUrl: 'http://api.test', fetch }),
		oncaptured,
		onmanual,
		connectEvents: fakeEvents
	});
	return { fetch, oncaptured, onmanual, user: userEvent.setup() };
}

async function submitUrl(user: ReturnType<typeof userEvent.setup>, url = URL_TEXT) {
	await user.type(screen.getByLabelText('Job posting URL'), url);
	await user.click(screen.getByRole('button', { name: 'Capture' }));
}

describe('CaptureDialog', () => {
	it('shows Required on a blank submit and sends nothing', async () => {
		const { fetch, user } = setup(() => json(capture));
		expect(screen.getByRole('dialog', { name: 'Capture from URL' })).toBeInTheDocument();
		expect(screen.getByLabelText('Job posting URL')).toHaveFocus();
		await user.click(screen.getByRole('button', { name: 'Capture' }));
		expect(await screen.findByText('Required')).toBeInTheDocument();
		expect(screen.getByLabelText('Job posting URL')).toHaveAttribute('aria-invalid', 'true');
		expect(fetch).not.toHaveBeenCalled();
	});

	it('disables the button while capturing, then hands a 200 capture over', async () => {
		let answer!: (response: Response) => void;
		const { oncaptured, user } = setup(() => new Promise((resolve) => (answer = resolve)));
		await submitUrl(user);

		const button = await screen.findByRole('button', { name: 'Capturing…' });
		expect(button).toBeDisabled();
		expect(screen.getByLabelText('Job posting URL')).toBeDisabled();
		expect(screen.getByText('Reading the page…')).toBeInTheDocument();

		answer(json({ ...capture, method: 'json_ld' }));
		await waitFor(() =>
			expect(oncaptured).toHaveBeenCalledWith({
				...capture,
				method: 'json_ld'
			})
		);
	});

	it('shows a 422 or 502 message on the URL field', async () => {
		const answers = [
			json({ errors: [{ field: 'url', message: 'Must be an http or https URL' }] }, 422),
			json({ reason: 'not_html', message: "The link isn't a web page." }, 502)
		];
		const { oncaptured, user } = setup(() => answers.shift()!);
		await submitUrl(user);
		expect(await screen.findByText('Must be an http or https URL')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Capture' }));
		expect(await screen.findByText("The link isn't a web page.")).toBeInTheDocument();
		expect(screen.getByLabelText('Job posting URL')).toHaveAttribute('aria-invalid', 'true');
		expect(oncaptured).not.toHaveBeenCalled();
	});

	it('shows the API error text on a network failure', async () => {
		const { user } = setup(() => Promise.reject(new TypeError('offline')));
		await submitUrl(user);
		expect(
			await screen.findByText("Couldn't capture the page. Check that the API is running.")
		).toBeInTheDocument();
	});

	it('follows a queued capture until the job succeeds', async () => {
		const { oncaptured, user } = setup(() => json({ job_id: 7 }, 202));
		await submitUrl(user);
		expect(await screen.findByText('Asking Claude to read the posting…')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Capturing…' })).toBeDisabled();

		const source = FakeEventSource.last!;
		source.emit('job.started', {
			job: job({
				status: 'running',
				attempts: 1,
				updated_at: '2026-10-22T00:00:01Z'
			})
		});
		source.emit('job.succeeded', {
			job: job({
				status: 'succeeded',
				result: capture,
				updated_at: '2026-10-22T00:00:02Z'
			})
		});
		await waitFor(() => expect(oncaptured).toHaveBeenCalledWith(capture));
		expect(source.closed).toBe(true);
	});

	it('shows the usage-limit text for a deferred job', async () => {
		const { user } = setup(() => json({ job_id: 7 }, 202));
		await submitUrl(user);
		await screen.findByText('Asking Claude to read the posting…');
		FakeEventSource.last!.emit('job.deferred', {
			job: job({
				last_error: 'Claude usage limit hit',
				updated_at: '2026-10-22T00:00:01Z'
			})
		});
		expect(
			await screen.findByText("Claude's usage limit was hit. This continues when it resets.")
		).toBeInTheDocument();
	});

	it('offers Add by hand when the job fails', async () => {
		const { onmanual, oncaptured, user } = setup(() => json({ job_id: 7 }, 202));
		await submitUrl(user);
		await screen.findByText('Asking Claude to read the posting…');
		FakeEventSource.last!.emit('job.failed', {
			job: job({
				status: 'failed',
				attempts: 3,
				updated_at: '2026-10-22T00:00:01Z'
			})
		});
		expect(
			await screen.findByText(
				"Claude couldn't read this page. Try again, or add the details by hand."
			)
		).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Add by hand' }));
		expect(onmanual).toHaveBeenCalledWith(URL_TEXT);
		expect(oncaptured).not.toHaveBeenCalled();
	});
});
