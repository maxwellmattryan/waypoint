import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import FitBadge from './FitBadge.svelte';

describe('FitBadge', () => {
	it.each([
		[82, 'strong', '82 · Strong fit'],
		[55, 'partial', '55 · Partial fit'],
		[12, 'weak', '12 · Weak fit']
	] as const)('shows %i as %s', (score, band, label) => {
		render(FitBadge, { score, band });
		expect(screen.getByText(label)).toBeInTheDocument();
		expect(screen.queryByText('Needs review')).toBeNull();
	});

	it('adds Needs review when a newer score waits', () => {
		render(FitBadge, { score: 70, band: 'strong', inReview: true });
		expect(screen.getByText('70 · Strong fit')).toBeInTheDocument();
		expect(screen.getByText('Needs review')).toBeInTheDocument();
	});

	it('shows only Needs review without an applied score', () => {
		render(FitBadge, { score: null, band: null, inReview: true });
		expect(screen.getByText('Needs review')).toBeInTheDocument();
		expect(screen.queryByText(/fit$/)).toBeNull();
	});

	it('renders nothing for an unscored posting', () => {
		const { container } = render(FitBadge, { score: null, band: null });
		expect(container.textContent?.trim()).toBe('');
	});
});
