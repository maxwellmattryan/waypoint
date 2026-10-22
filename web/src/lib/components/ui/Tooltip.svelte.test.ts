import { act, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import TooltipHarness from './__fixtures__/TooltipHarness.svelte';

afterEach(() => vi.useRealTimers());

function trigger() {
	return screen.getByRole('button', { name: 'Archive' });
}

async function expectHidden() {
	await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
	expect(trigger()).not.toHaveAttribute('aria-describedby');
}

function expectVisible() {
	const tooltip = screen.getByRole('tooltip');
	expect(tooltip).toHaveTextContent('Archive application');
	expect(tooltip.id).not.toBe('');
	expect(trigger()).toHaveAttribute('aria-describedby', tooltip.id);
}

describe('Tooltip', () => {
	it('is hidden at first', () => {
		render(TooltipHarness);
		expect(screen.queryByRole('tooltip')).toBeNull();
		expect(trigger()).not.toHaveAttribute('aria-describedby');
	});

	it('shows after the delay on hover and hides on pointer leave', async () => {
		vi.useFakeTimers();
		render(TooltipHarness, { delay: 500 });
		await fireEvent.pointerEnter(trigger().parentElement as HTMLElement);
		await act(() => vi.advanceTimersByTimeAsync(499));
		expect(screen.queryByRole('tooltip')).toBeNull();
		await act(() => vi.advanceTimersByTimeAsync(1));
		expectVisible();

		await fireEvent.pointerLeave(trigger().parentElement as HTMLElement);
		vi.useRealTimers();
		await expectHidden();
	});

	it('shows at once on focus and hides on blur', async () => {
		render(TooltipHarness);
		await act(() => trigger().focus());
		expectVisible();
		await act(() => trigger().blur());
		await expectHidden();
	});

	it('hides on Escape without letting it propagate', async () => {
		const onWindowKeydown = vi.fn();
		window.addEventListener('keydown', onWindowKeydown);
		render(TooltipHarness);
		await act(() => trigger().focus());
		expectVisible();
		await fireEvent.keyDown(trigger(), { key: 'Escape' });
		await expectHidden();
		expect(onWindowKeydown).not.toHaveBeenCalled();
		window.removeEventListener('keydown', onWindowKeydown);
	});
});
