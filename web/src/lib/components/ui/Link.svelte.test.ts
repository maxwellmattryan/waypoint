import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { Link } from './index';
import { text } from './__fixtures__/snippets';

describe('Link', () => {
	it('renders an internal link without target or rel', () => {
		render(Link, { href: '/jobs', children: text('Jobs') });
		const link = screen.getByRole('link', { name: 'Jobs' });
		expect(link).toHaveAttribute('href', '/jobs');
		expect(link).not.toHaveAttribute('target');
		expect(link).not.toHaveAttribute('rel');
	});

	it('opens external links in a new tab with noopener noreferrer', () => {
		render(Link, { href: 'https://example.com', external: true, children: text('Docs') });
		const link = screen.getByRole('link', { name: 'Docs' });
		expect(link).toHaveAttribute('target', '_blank');
		expect(link).toHaveAttribute('rel', 'noopener noreferrer');
	});

	it('renders a disabled link with no href, aria-disabled and out of the tab order', () => {
		render(Link, { href: '/jobs', disabled: true, children: text('Jobs') });
		const link = screen.getByText('Jobs').closest('a');
		expect(link).not.toBeNull();
		expect(link).not.toHaveAttribute('href');
		expect(link).toHaveAttribute('aria-disabled', 'true');
		expect(link).toHaveAttribute('tabindex', '-1');
	});
});
