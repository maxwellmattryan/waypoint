import { render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { text } from '#lib/components/ui/__fixtures__/snippets.js';
import AppShell from './AppShell.svelte';

const current = vi.hoisted(() => ({ url: new URL('http://127.0.0.1/') }));

vi.mock('$app/state', () => ({ page: current }));

function renderAt(pathname: string) {
	current.url = new URL(pathname, 'http://127.0.0.1');
	render(AppShell, { connection: 'live', children: text('Body') });
	const nav = screen.getByRole('navigation', { name: 'Main' });
	return {
		dashboard: screen.getByRole('link', { name: 'Dashboard' }),
		applications: screen.getByRole('link', { name: 'Applications' }),
		nav
	};
}

describe('AppShell', () => {
	it.each([
		['/', 'dashboard'],
		['/applications', 'applications'],
		['/applications/42', 'applications'],
		['/dev/primitives', null]
	] as const)('marks the current section at %s', (pathname, section) => {
		const links = renderAt(pathname);
		expect(links.nav).toContainElement(links.dashboard);
		expect(links.dashboard).toHaveAttribute('href', '/');
		expect(links.applications).toHaveAttribute('href', '/applications');

		for (const name of ['dashboard', 'applications'] as const) {
			if (name === section) expect(links[name]).toHaveAttribute('aria-current', 'page');
			else expect(links[name]).not.toHaveAttribute('aria-current');
		}
	});

	it('does not treat a path that only starts with the same letters as Applications', () => {
		const links = renderAt('/applicationsx');
		expect(links.applications).not.toHaveAttribute('aria-current');
	});
});
