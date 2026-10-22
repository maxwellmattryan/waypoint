import { act, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Toaster, toast } from './index';

beforeEach(() => {
	vi.useFakeTimers();
	render(Toaster);
});

afterEach(() => {
	toast.clear();
	vi.useRealTimers();
});

/** Lets Svelte flush and the stubbed transitions finish. */
async function settle() {
	await act(() => vi.advanceTimersByTimeAsync(0));
}

describe('toast', () => {
	it('renders a shown toast with its title and description', async () => {
		toast.show({ title: 'Saved', description: 'Acme · Staff engineer' });
		await settle();
		const item = screen.getByRole('status');
		expect(item).toHaveTextContent('Saved');
		expect(item).toHaveTextContent('Acme · Staff engineer');
		expect(item.closest('[aria-live="polite"]')).not.toBeNull();
	});

	it('auto-dismisses after its duration, and duration 0 stays', async () => {
		toast.show({ title: 'Brief', duration: 1000 });
		toast.show({ title: 'Sticky', duration: 0 });
		await settle();
		expect(screen.getAllByRole('status')).toHaveLength(2);

		await act(() => vi.advanceTimersByTimeAsync(999));
		expect(screen.getByText('Brief')).toBeInTheDocument();
		await act(() => vi.advanceTimersByTimeAsync(1));
		await waitFor(() => expect(screen.queryByText('Brief')).toBeNull());

		await act(() => vi.advanceTimersByTimeAsync(60_000));
		expect(screen.getByText('Sticky')).toBeInTheDocument();
		expect(toast.items.map((t) => t.title)).toEqual(['Sticky']);
	});

	it('defaults to a 5000ms duration', async () => {
		toast.show({ title: 'Default' });
		await settle();
		await act(() => vi.advanceTimersByTimeAsync(4999));
		expect(toast.items).toHaveLength(1);
		await act(() => vi.advanceTimersByTimeAsync(1));
		expect(toast.items).toHaveLength(0);
	});

	it('is removed by its Dismiss button and by toast.dismiss(id)', async () => {
		toast.show({ title: 'First', duration: 0 });
		const second = toast.show({ title: 'Second', duration: 0 });
		await settle();

		const dismissButtons = screen.getAllByRole('button', { name: 'Dismiss notification' });
		await fireEvent.click(dismissButtons[0]);
		await waitFor(() => expect(screen.queryByText('First')).toBeNull());

		toast.dismiss(second);
		await waitFor(() => expect(screen.queryByText('Second')).toBeNull());
		expect(toast.items).toEqual([]);
	});

	it('gives danger toasts role="alert" and other tones role="status"', async () => {
		for (const tone of ['neutral', 'success', 'info', 'warning'] as const) {
			toast.show({ title: tone, tone, duration: 0 });
		}
		toast.show({ title: 'Failed', tone: 'danger', duration: 0 });
		await settle();
		expect(screen.getAllByRole('status')).toHaveLength(4);
		expect(screen.getByRole('alert')).toHaveTextContent('Failed');
	});

	it('success and error set the success and danger tones', async () => {
		toast.success('Sent', { duration: 0 });
		toast.error('Not sent', { description: 'Try again', duration: 0 });
		await settle();
		expect(toast.items.map((t) => [t.title, t.tone])).toEqual([
			['Sent', 'success'],
			['Not sent', 'danger']
		]);
		expect(screen.getByRole('status')).toHaveTextContent('Sent');
		expect(screen.getByRole('alert')).toHaveTextContent('Try again');
	});
});
