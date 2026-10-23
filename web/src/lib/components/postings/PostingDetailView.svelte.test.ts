import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { JobPostingDetail } from '#lib/api';
import PostingDetailView from './PostingDetailView.svelte';

function posting(overrides: Partial<JobPostingDetail> = {}): JobPostingDetail {
	return {
		id: 9,
		source: 'lever',
		board: 'example-labs',
		external_id: 'abc',
		title: 'Staff Engineer',
		company: 'Example Labs',
		url: 'https://jobs.lever.co/example-labs/abc',
		apply_url: 'https://jobs.lever.co/example-labs/abc/apply',
		department: null,
		employment_type: null,
		location_raw: null,
		city: null,
		region: null,
		country: null,
		remote: false,
		posted_at: null,
		first_seen_at: '2026-10-22T00:00:00Z',
		last_seen_at: '2026-10-22T00:00:00Z',
		created_at: '2026-10-22T00:00:00Z',
		updated_at: '2026-10-22T00:00:00Z',
		removed_at: null,
		fit_score: null,
		fit_band: null,
		fit_in_review: false,
		description: 'About the role',
		...overrides
	};
}

/** The `<dd>` beside the `<dt>` named `key`. */
function value(key: string): string | null | undefined {
	return screen.getByText(key, { selector: 'dt' }).nextElementSibling?.textContent?.trim();
}

describe('PostingDetailView', () => {
	it('renders markup in the description as literal text', () => {
		const { container } = render(PostingDetailView, {
			posting: posting({ description: 'Line one\n<b>x</b><script>alert(1)</script>' })
		});
		expect(container.querySelector('b')).toBeNull();
		expect(container.querySelector('script')).toBeNull();
		expect(screen.getByText(/<b>x<\/b><script>/)).toHaveTextContent(
			'Line one <b>x</b><script>alert(1)</script>'
		);
	});

	it('shows a dash for null fields and the parsed location joined', () => {
		render(PostingDetailView, { posting: posting({ city: 'Berlin', country: 'Germany' }) });
		expect(screen.getByRole('heading', { level: 1, name: 'Staff Engineer' })).toBeInTheDocument();
		expect(value('Board')).toBe('Lever · example-labs');
		for (const key of ['Department', 'Employment type', 'Location', 'Posted']) {
			expect(value(key)).toBe('—');
		}
		expect(value('Parsed location')).toBe('Berlin, Germany');
		expect(value('Remote')).toBe('No');
		expect(screen.queryByText('Removed', { selector: 'dt' })).toBeNull();
	});

	it('shows the removed date and badges, and the empty description text', () => {
		render(PostingDetailView, {
			posting: posting({ removed_at: '2026-10-23T00:00:00Z', remote: true, description: '' })
		});
		expect(value('Removed')).not.toBe('—');
		expect(value('Remote')).toBe('Yes');
		expect(screen.getByText('Removed', { selector: 'span' })).toBeInTheDocument();
		expect(screen.getByText('Remote', { selector: 'span' })).toBeInTheDocument();
		expect(screen.getByText('The board gave no description.')).toBeInTheDocument();
	});

	it('renders the external links in a new tab', () => {
		render(PostingDetailView, { posting: posting() });
		for (const [name, href] of [
			['View posting', 'https://jobs.lever.co/example-labs/abc'],
			['Apply', 'https://jobs.lever.co/example-labs/abc/apply']
		]) {
			const link = screen.getByRole('link', { name });
			expect(link).toHaveAttribute('href', href);
			expect(link).toHaveAttribute('target', '_blank');
			expect(link).toHaveAttribute('rel', 'noopener noreferrer');
		}
	});

	it.each([
		['a non-http(s) URL', { url: 'javascript:alert(1)', apply_url: 'ftp://example.test/x' }],
		['an apply URL equal to the posting URL', { apply_url: 'https://jobs.lever.co/example-labs/abc' }],
		['no apply URL', { apply_url: null }]
	])('renders no Apply link for %s', (_, overrides) => {
		render(PostingDetailView, { posting: posting(overrides) });
		expect(screen.queryByRole('link', { name: 'Apply' })).toBeNull();
	});

	it('renders no View posting link for a non-http(s) URL', () => {
		render(PostingDetailView, { posting: posting({ url: 'javascript:alert(1)', apply_url: null }) });
		expect(screen.queryByRole('link')).toBeNull();
	});
});
