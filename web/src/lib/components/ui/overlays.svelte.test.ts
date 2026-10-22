import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import NestedHarness from './__fixtures__/NestedHarness.svelte';
import OverlayHarness from './__fixtures__/OverlayHarness.svelte';

const rows: { kind: 'dialog' | 'drawer' }[] = [{ kind: 'dialog' }, { kind: 'drawer' }];

function backdrop(): HTMLElement {
	const el = document.querySelector<HTMLElement>('[data-backdrop]');
	expect(el).not.toBeNull();
	return el as HTMLElement;
}

type Kind = 'dialog' | 'drawer';

async function setup(props: { kind: Kind; closeOnBackdrop?: boolean; autofocus?: boolean }) {
	const user = userEvent.setup();
	const onclose = vi.fn();
	render(OverlayHarness, { onclose, ...props });
	const trigger = screen.getByRole('button', { name: 'Open' });
	await user.click(trigger);
	const panel = await screen.findByRole('dialog', { name: 'Edit application' });
	return { user, onclose, trigger, panel };
}

async function expectClosed(trigger: HTMLElement, onclose: ReturnType<typeof vi.fn>) {
	await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
	expect(screen.getByTestId('state')).toHaveAttribute('data-open', 'false');
	expect(onclose).toHaveBeenCalledTimes(1);
	expect(trigger).toHaveFocus();
}

describe.each(rows)('modal overlays: $kind', ({ kind }) => {
	it('is a labelled modal dialog and focuses its first focusable element', async () => {
		const { panel } = await setup({ kind });
		expect(panel).toHaveAttribute('aria-modal', 'true');
		expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus();
	});

	it('focuses the [data-autofocus] element when present', async () => {
		await setup({ kind, autofocus: true });
		expect(screen.getByRole('button', { name: 'Last' })).toHaveFocus();
	});

	it('wraps Tab and Shift+Tab inside the panel', async () => {
		const { user } = await setup({ kind });
		const close = screen.getByRole('button', { name: 'Close' });
		const last = screen.getByRole('button', { name: 'Last' });
		await user.tab({ shift: true });
		expect(last).toHaveFocus();
		await user.tab();
		expect(close).toHaveFocus();
		await user.tab();
		expect(screen.getByRole('button', { name: 'First' })).toHaveFocus();
	});

	it('closes on Escape and restores focus', async () => {
		const { user, trigger, onclose } = await setup({ kind });
		await user.keyboard('{Escape}');
		await expectClosed(trigger, onclose);
	});

	it('closes from the Close button', async () => {
		const { user, trigger, onclose } = await setup({ kind });
		await user.click(screen.getByRole('button', { name: 'Close' }));
		await expectClosed(trigger, onclose);
	});

	it('closes on a backdrop click', async () => {
		const { user, trigger, onclose } = await setup({ kind });
		await user.click(backdrop());
		await expectClosed(trigger, onclose);
	});

	it('stays open on a panel click, and on a backdrop click with closeOnBackdrop off', async () => {
		const { user, panel, onclose } = await setup({ kind, closeOnBackdrop: false });
		await user.click(panel);
		await user.click(backdrop());
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByTestId('state')).toHaveAttribute('data-open', 'true');
		expect(onclose).not.toHaveBeenCalled();
	});
});

describe('nested overlays', () => {
	it('nested overlays: Escape closes topmost, scroll lock released last', async () => {
		const user = userEvent.setup();
		document.body.style.overflow = 'scroll';
		render(NestedHarness);

		await user.click(screen.getByRole('button', { name: 'Open drawer' }));
		await screen.findByRole('dialog', { name: 'Application' });
		expect(document.body.style.overflow).toBe('hidden');

		await user.click(screen.getByRole('button', { name: 'Open dialog' }));
		await screen.findByRole('dialog', { name: 'Confirm' });
		expect(document.body.style.overflow).toBe('hidden');

		await user.keyboard('{Escape}');
		await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Confirm' })).toBeNull());
		expect(screen.getByRole('dialog', { name: 'Application' })).toBeInTheDocument();
		expect(document.body.style.overflow).toBe('hidden');

		await user.keyboard('{Escape}');
		await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
		expect(document.body.style.overflow).toBe('scroll');
		document.body.style.overflow = '';
	});
});
