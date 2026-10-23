import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ApplicationPacket, PendingAction, ResumeVersion } from '#lib/api';
import { makePacket } from '#lib/packets/__fixtures__/packets.js';
import type { ApprovalInput, Result } from '#lib/packets/api.js';
import type { ResumesState } from '#lib/packets/packets.svelte.js';
import PacketAnswerEditor from './PacketAnswerEditor.svelte';

function resume(id: number, name: string): ResumeVersion {
	return {
		id,
		name,
		filename: `${name}.pdf`,
		size_bytes: 9,
		sha256: 'a'.repeat(64),
		has_text: true,
		application_count: 0,
		created_at: '2026-10-22T10:00:00Z'
	};
}

const loaded: ResumesState = {
	kind: 'loaded',
	resumes: [resume(1, 'Backend'), resume(2, 'Frontend')]
};

function setup(
	packet: ApplicationPacket = makePacket(),
	resumes: ResumesState = loaded,
	result: Result<PendingAction> = { kind: 'conflict' }
) {
	const onsubmit = vi.fn<(input: ApprovalInput) => Promise<Result<PendingAction>>>(
		async () => result
	);
	const props = {
		packet,
		resumes,
		onsubmit,
		onretry: vi.fn(),
		oncancel: vi.fn()
	};
	render(PacketAnswerEditor, props);
	return props;
}

const select = () => screen.getByRole<HTMLSelectElement>('combobox', { name: 'Resume' });
const answer = (question: string) =>
	screen.getByRole<HTMLTextAreaElement>('textbox', { name: question });
const submit = () => userEvent.click(screen.getByRole('button', { name: 'Approve with edits' }));

describe('PacketAnswerEditor', () => {
	it('preselects the draft resume and prefills the answers', () => {
		const packet = makePacket();
		packet.answers[2] = { ...packet.answers[2], answer: null, citations: [] };
		setup(packet);
		expect(select().value).toBe('1');
		expect(answer('When could you start?').value).toBe('Four weeks after an offer.');
		expect(answer('What are your salary expectations?').value).toBe('');
		expect(screen.getAllByRole('textbox')).toHaveLength(5);
	});

	it('sends the edited resume and trimmed answers, a blank one as null', async () => {
		const props = setup();
		await userEvent.selectOptions(select(), '2');
		const notice = answer('When could you start?');
		await userEvent.clear(notice);
		await userEvent.type(notice, '  In two weeks.  ');
		await userEvent.clear(answer('Why are you looking to leave your current role?'));
		await submit();
		expect(props.onsubmit).toHaveBeenCalledWith({
			packet_id: 3,
			resume_version_id: 2,
			answers: {
				work_authorization: 'Yes. I am an EU citizen.',
				notice_period: 'In two weeks.',
				salary_expectation: 'Around 90,000 EUR.',
				why_leaving: null,
				why_company: 'You build Rust services.'
			}
		});
	});

	it('blocks submit with "Choose a resume" when the draft resume was deleted', async () => {
		const props = setup(makePacket({ resume_version_id: 9 }));
		expect(select().value).toBe('');
		expect(screen.getByRole('option', { name: 'Choose a resume' })).toBeInTheDocument();
		await submit();
		expect(screen.getByText('Choose a resume', { selector: 'p' })).toBeInTheDocument();
		expect(props.onsubmit).not.toHaveBeenCalled();
	});

	it('blocks submit with "At most 1,500 characters" on a long answer', async () => {
		const props = setup();
		const notice = answer('When could you start?');
		await userEvent.clear(notice);
		await userEvent.click(notice);
		await userEvent.paste('a'.repeat(1_501));
		await submit();
		expect(screen.getByText('At most 1,500 characters')).toBeInTheDocument();
		expect(props.onsubmit).not.toHaveBeenCalled();
	});

	it("shows the server's 422 message above the buttons", async () => {
		setup(makePacket(), loaded, {
			kind: 'invalid',
			errors: [
				{
					field: 'input',
					message: '$.resume_version_id: no resume has this id'
				}
			]
		});
		await submit();
		expect(screen.getByRole('alert')).toHaveTextContent(
			'$.resume_version_id: no resume has this id'
		);
	});

	it('disables the Select while resumes load, and offers Try again when they fail', async () => {
		const { unmount } = render(PacketAnswerEditor, {
			packet: makePacket(),
			resumes: { kind: 'loading' },
			onsubmit: vi.fn(),
			onretry: vi.fn(),
			oncancel: vi.fn()
		});
		expect(select()).toBeDisabled();
		expect(screen.getByText('Loading resumes…')).toBeInTheDocument();
		unmount();
		const props = setup(makePacket(), { kind: 'error' });
		expect(screen.getByText("Couldn't load resumes")).toBeInTheDocument();
		await userEvent.click(screen.getByRole('button', { name: 'Try again' }));
		expect(props.onretry).toHaveBeenCalledTimes(1);
	});

	it('cancels', async () => {
		const props = setup();
		await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
		expect(props.oncancel).toHaveBeenCalledTimes(1);
	});
});
