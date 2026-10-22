import { expect, test, type Page } from '@playwright/test';

// The runs routes are mocked, so the test doesn't depend on runs in the Compose database.
// Screenshots go to $WAYPOINT_SCREENSHOTS when set (Forge points it at the job's folder).
const screenshots = process.env.WAYPOINT_SCREENSHOTS ?? 'test-results/screenshots';

const startedAt = '2026-10-22T10:00:00Z';

const run = {
	id: 1,
	workflow: 'triage_email',
	status: 'succeeded',
	input: { message_id: 'm-42' },
	output: { status: 'interviewing', confidence: 0.92 },
	error: null,
	input_tokens: 12345,
	output_tokens: 678,
	cost_usd: 0.0421,
	job_id: 7,
	created_at: startedAt,
	updated_at: '2026-10-22T10:00:13Z',
	started_at: startedAt,
	finished_at: '2026-10-22T10:00:12.300Z'
};

const step = (id: number, ordinal: number, name: string, extra: object = {}) => ({
	id,
	run_id: 1,
	ordinal,
	name,
	status: 'succeeded',
	attempts: 1,
	input: { message_id: 'm-42' },
	output: { ok: true },
	error: null,
	input_tokens: 6000,
	output_tokens: 300,
	cost_usd: 0.02,
	claude_call_id: null,
	created_at: startedAt,
	updated_at: startedAt,
	started_at: startedAt,
	finished_at: '2026-10-22T10:00:06Z',
	tool_calls: [],
	...extra
});

const detail = {
	...run,
	steps: [
		step(10, 0, 'classify', {
			attempts: 2,
			tool_calls: [
				{
					id: 100,
					run_step_id: 10,
					tool: 'lookup_application',
					permission: 'read',
					status: 'succeeded',
					input: { company: 'Acme' },
					output: { application_id: 3 },
					error: null,
					input_tokens: 0,
					output_tokens: 0,
					cost_usd: 0,
					claude_call_id: null,
					duration_ms: 850,
					created_at: startedAt,
					finished_at: startedAt
				}
			]
		}),
		step(11, 1, 'update_status')
	]
};

async function mockRuns(page: Page) {
	await page.route('**/api/runs*', (route) => route.fulfill({ json: [run] }));
	await page.route('**/api/runs/1', (route) => route.fulfill({ json: detail }));
	await page.route('**/api/runs/999', (route) => route.fulfill({ status: 404, body: '' }));
}

test('runs: list, detail with a tool call, and not found', async ({ page }) => {
	await mockRuns(page);
	// No transitions, so screenshots never catch a panel mid-slide.
	await page.emulateMedia({ reducedMotion: 'reduce' });
	const nav = page.getByRole('navigation', { name: 'Main' });

	await page.goto('/runs');
	await expect(nav.getByRole('link', { name: 'Runs' })).toHaveAttribute('aria-current', 'page');
	const card = page.getByRole('list', { name: 'Runs' }).getByRole('link');
	await expect(card).toHaveCount(1);
	await expect(card).toContainText('triage_email');
	await expect(card).toContainText('#1');
	await expect(card).toContainText('12,345 in · 678 out');
	await expect(card).toContainText('$0.04');
	await page.screenshot({ path: `${screenshots}/runs-list.png`, fullPage: true });

	await card.click();
	await expect(page).toHaveURL(/\/runs\/1$/);
	await expect(nav.getByRole('link', { name: 'Runs' })).toHaveAttribute('aria-current', 'page');
	await expect(page.getByRole('heading', { level: 1, name: 'triage_email' })).toBeVisible();

	const steps = page.getByRole('list', { name: 'Steps' });
	await expect(steps.getByRole('heading', { level: 3 })).toHaveText(['classify', 'update_status']);
	await expect(steps).toContainText('2 attempts');

	const call = page.locator('[data-tool-call="100"]');
	await expect(call).toContainText('lookup_application');
	await expect(call.getByLabel('Permission: read')).toBeVisible();
	await expect(call).toContainText('850 ms');
	const toggle = call.getByRole('button', { name: 'Show details' });
	await expect(toggle).toHaveAttribute('aria-expanded', 'false');
	await toggle.click();
	await expect(call.getByRole('button', { name: 'Hide details' })).toHaveAttribute(
		'aria-expanded',
		'true'
	);
	await expect(call.getByLabel('lookup_application input')).toContainText('"company": "Acme"');
	await expect(call.getByLabel('lookup_application output')).toContainText('"application_id": 3');
	await expect(call.getByText('No error')).toBeVisible();

	await expect(page.getByRole('heading', { name: 'Memories used' })).toBeVisible();
	await expect(page.getByText('No memories used')).toBeVisible();

	await page.mouse.move(0, 0);
	await page.screenshot({ path: `${screenshots}/run-detail.png`, fullPage: true });
	await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
	await page.screenshot({ path: `${screenshots}/run-detail-dark.png`, fullPage: true });
	await page.evaluate(() => document.documentElement.removeAttribute('data-theme'));

	for (const id of ['999', 'abc']) {
		await page.goto(`/runs/${id}`);
		await expect(page.getByText('Run not found')).toBeVisible();
	}
	await page.getByRole('link', { name: 'All runs' }).click();
	await expect(page).toHaveURL(/\/runs$/);
});
