import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { Badge, Card, EmptyState, JsonView, KeyValue, Skeleton, Stat } from './index';
import { badgeTones, statTones, type BadgeTone } from './styles';
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

describe('Stat', () => {
	it('renders its label, value and detail', () => {
		render(Stat, {
			label: 'Cost',
			value: '$1.25',
			detail: 'across 3 steps',
			'data-testid': 's'
		});
		const stat = screen.getByTestId('s');
		expect(stat).toHaveTextContent('Cost');
		expect(stat).toHaveTextContent('$1.25');
		expect(stat).toHaveTextContent('across 3 steps');
		expect(stat.querySelector('button, a')).toBeNull();
	});

	it('colours the value by tone', () => {
		render(Stat, { label: 'Status', value: 'Failed', tone: 'danger' });
		expect(screen.getByText('Failed')).toHaveClass(statTones.danger);
	});
});

describe('KeyValue', () => {
	it('renders each item as a term and its description, text or snippet', () => {
		render(KeyValue, {
			items: [
				{ key: 'Tool', value: 'lookup' },
				{ key: 'Permission', value: text('read') }
			]
		});
		const terms = screen.getAllByRole('term').map((el) => el.textContent);
		const values = screen.getAllByRole('definition').map((el) => el.textContent?.trim());
		expect(terms).toEqual(['Tool', 'Permission']);
		expect(values).toEqual(['lookup', 'read']);
	});
});

describe('JsonView', () => {
	it('pretty-prints JSON in a focusable, labelled block', () => {
		render(JsonView, { value: { q: 'hi', n: [1] }, label: 'Input' });
		const block = screen.getByLabelText('Input');
		expect(block.tagName).toBe('PRE');
		expect(block).toHaveAttribute('tabindex', '0');
		expect(block.textContent).toBe('{\n  "q": "hi",\n  "n": [\n    1\n  ]\n}');
	});

	it('renders markup in a value as literal text', () => {
		const value = '<img src=x onerror=alert(1)>';
		render(JsonView, { value, label: 'Output' });
		const block = screen.getByLabelText('Output');
		expect(block.textContent).toBe(JSON.stringify(value));
		expect(block.children).toHaveLength(0);
		expect(document.querySelector('img')).toBeNull();
	});

	it.each([null, undefined])('shows its empty text for %s', (value) => {
		render(JsonView, { value, label: 'Output', empty: 'No output yet' });
		expect(screen.getByText('No output yet')).toBeInTheDocument();
		expect(screen.queryByLabelText('Output')).toBeNull();
	});

	it('defaults the empty text to "None"', () => {
		render(JsonView, { value: null, label: 'Error' });
		expect(screen.getByText('None')).toBeInTheDocument();
	});
});
