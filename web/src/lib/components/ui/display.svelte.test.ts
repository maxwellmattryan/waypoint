import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { Badge, Card, EmptyState, Skeleton } from './index';
import { badgeTones, type BadgeTone } from './styles';
import { html, text } from './__fixtures__/snippets';

describe('Card', () => {
	it('renders a div without href', () => {
		render(Card, { 'data-testid': 'card', children: text('Body') });
		const card = screen.getByTestId('card');
		expect(card.tagName).toBe('DIV');
		expect(screen.queryByRole('link')).toBeNull();
	});

	it('renders a link with href', () => {
		render(Card, { href: '/jobs/1', children: text('Acme') });
		expect(screen.getByRole('link', { name: 'Acme' })).toHaveAttribute('href', '/jobs/1');
	});
});

describe('Badge', () => {
	it.each(Object.keys(badgeTones) as BadgeTone[])('renders children with the %s tone', (tone) => {
		render(Badge, { tone, 'data-testid': tone, children: text(`${tone} badge`) });
		const badge = screen.getByTestId(tone);
		expect(badge.tagName).toBe('SPAN');
		expect(badge).toHaveTextContent(`${tone} badge`);
		for (const cls of badgeTones[tone].split(' ')) expect(badge).toHaveClass(cls);
	});

	it('defaults to the neutral tone', () => {
		render(Badge, { 'data-testid': 'b', children: text('Draft') });
		expect(screen.getByTestId('b')).toHaveClass(...badgeTones.neutral.split(' '));
	});
});

describe('EmptyState', () => {
	it('renders its title, description and action', () => {
		render(EmptyState, {
			title: 'No applications yet',
			description: 'Add one to start tracking.',
			action: html('<button type="button">Add application</button>')
		});
		expect(screen.getByText('No applications yet')).toBeInTheDocument();
		expect(screen.getByText('Add one to start tracking.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Add application' })).toBeInTheDocument();
	});
});

describe('Skeleton', () => {
	it('is hidden from assistive tech and pulses only when motion is allowed', () => {
		render(Skeleton, { 'data-testid': 's', class: 'h-8 w-32' });
		const skeleton = screen.getByTestId('s');
		expect(skeleton).toHaveAttribute('aria-hidden', 'true');
		const classes = skeleton.className.split(/\s+/);
		const animated = classes.filter((c) => c.includes('animate-'));
		expect(animated).toEqual(['motion-safe:animate-pulse']);
	});
});
