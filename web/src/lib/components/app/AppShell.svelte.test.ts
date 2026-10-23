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
		goals: screen.getByRole('link', { name: 'Goals' }),
		postings: screen.getByRole('link', { name: 'Postings' }),
		resumes: screen.getByRole('link', { name: 'Resumes' }),
		memory: screen.getByRole('link', { name: 'Memory' }),
		review: screen.getByRole('link', { name: 'Review' }),
		nav
	};
}

describe('AppShell', () => {
	it.each([
		['/', 'dashboard'],
		['/applications', 'applications'],
		['/applications/42', 'applications'],
		['/goals', 'goals'],
		['/goals/3', 'goals'],
		['/postings', 'postings'],
		['/postings/9', 'postings'],
		['/resumes', 'resumes'],
		['/resumes/7', 'resumes'],
		['/memories', 'memory'],
		['/review', 'review'],
		['/runs', null],
		['/dev/primitives', null]
	] as const)('marks the current section at %s', (pathname, section) => {
		const links = renderAt(pathname);
		expect(links.nav).toContainElement(links.dashboard);
		expect(links.nav).toContainElement(links.resumes);
		expect(links.dashboard).toHaveAttribute('href', '/');
		expect(links.applications).toHaveAttribute('href', '/applications');
		expect(links.resumes).toHaveAttribute('href', '/resumes');
		expect(links.nav).toContainElement(links.goals);
		expect(links.goals).toHaveAttribute('href', '/goals');
		// Goals comes right before Postings.
		expect(links.goals.nextElementSibling).toBe(links.postings);
		expect(links.nav).toContainElement(links.postings);
		expect(links.postings).toHaveAttribute('href', '/postings');
		expect(links.nav).toContainElement(links.review);
		expect(links.review).toHaveAttribute('href', '/review');
		expect(links.nav).toContainElement(links.memory);
		expect(links.memory).toHaveAttribute('href', '/memories');

		for (const name of [
			'dashboard',
			'applications',
			'goals',
			'postings',
			'resumes',
			'memory',
			'review'
		] as const) {
			if (name === section) expect(links[name]).toHaveAttribute('aria-current', 'page');
			else expect(links[name]).not.toHaveAttribute('aria-current');
		}
	});

	it('does not treat a path that only starts with the same letters as Applications', () => {
		const links = renderAt('/applicationsx');
		expect(links.applications).not.toHaveAttribute('aria-current');
	});

	it('does not treat a path that only starts with the same letters as Goals', () => {
		const links = renderAt('/goalsx');
		expect(links.goals).not.toHaveAttribute('aria-current');
	});

	it('does not treat a path that only starts with the same letters as Postings', () => {
		const links = renderAt('/postingsx');
		expect(links.postings).not.toHaveAttribute('aria-current');
	});

	it('does not treat a path that only starts with the same letters as Resumes', () => {
		const links = renderAt('/resumesx');
		expect(links.resumes).not.toHaveAttribute('aria-current');
	});
});
