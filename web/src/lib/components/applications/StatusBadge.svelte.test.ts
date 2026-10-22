import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { ApplicationStatus } from '#lib/api';
import { badgeTones, type BadgeTone } from '#lib/components/ui/styles.js';
import StatusBadge from './StatusBadge.svelte';

const cases: [ApplicationStatus, string, BadgeTone][] = [
	['sourced', 'Sourced', 'neutral'],
	['applied', 'Applied', 'info'],
	['interviewing', 'Interviewing', 'primary'],
	['offer', 'Offer', 'success'],
	['rejected', 'Rejected', 'danger']
];

describe('StatusBadge', () => {
	it.each(cases)('%s reads %s in the %s tone', (status, label, tone) => {
		render(StatusBadge, { status });
		const badge = screen.getByText(label);
		for (const cls of badgeTones[tone].split(' ')) expect(badge).toHaveClass(cls);
	});
});
