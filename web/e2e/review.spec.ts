import { expect, test, type APIRequestContext, type Page } from '@playwright/test';

// The whole MVP 3 flow through the real UI and API (the sample workflow calls no model). The
// Compose database is shared and persists between runs, so each test works on its own uniquely
// named application, finds its items by their unique text or run link, and never asserts on
// totals. Screenshots go to $WAYPOINT_SCREENSHOTS when set (Forge points it at the job's folder).
const screenshots = process.env.WAYPOINT_SCREENSHOTS ?? 'test-results/screenshots';

function collectErrors(page: Page): string[] {
	const errors: string[] = [];
	page.on('console', (message) => {
		if (message.type() === 'error') errors.push(message.text());
	});
	page.on('pageerror', (error) => errors.push(error.message));
	return errors;
}

async function setKillSwitch(request: APIRequestContext, paused: boolean) {
	const response = await request.put('/api/kill-switch', { data: { paused } });
	expect(response.status()).toBe(200);
}

async function createApplication(request: APIRequestContext, company: string): Promise<number> {
	const response = await request.post('/api/applications', {
		data: { company, title: 'Staff Engineer' }
	});
	expect(response.status()).toBe(201);
	return (await response.json()).id;
}

/** Starts the sample workflow from the application page and returns the run id from the toast. */
async function startRun(
	page: Page,
	applicationId: number,
	fields: { note: string; message: string; confidence: string }
): Promise<number> {
	await page.goto(`/applications/${applicationId}`);
	await page.getByRole('button', { name: 'Run sample workflow' }).click();
	const dialog = page.getByRole('dialog', { name: 'Run sample workflow' });
	await dialog.getByLabel('Note').fill(fields.note);
	await dialog.getByLabel('Follow-up message').fill(fields.message);
	await dialog.getByLabel('Confidence').fill(fields.confidence);
	await dialog.getByRole('button', { name: 'Start run' }).click();
	await expect(dialog).toBeHidden();
	const toast = page.getByText(/^Run #\d+ started$/);
	await expect(toast).toBeVisible();
	const match = /#(\d+)/.exec((await toast.textContent()) ?? '');
	expect(match).not.toBeNull();
	return Number(match?.[1]);
}

test('review: sample workflow pauses at the gate, approve, undo, inspect', async ({ page }) => {
	const errors = collectErrors(page);
	// No transitions, so screenshots never catch an item mid-fade.
	await page.emulateMedia({ reducedMotion: 'reduce' });
	const stamp = Date.now();
	const company = `E2E Review ${stamp}`;
	const note = `Ask about the team ${stamp}`;
	const message = `Thanks for your time ${stamp}`;

	await setKillSwitch(page.request, false);
	const applicationId = await createApplication(page.request, company);
	const runId = await startRun(page, applicationId, { note, message, confidence: '1' });

	// The queue holds the run's draft at the approval gate.
	const nav = page.getByRole('navigation', { name: 'Main' });
	await nav.getByRole('link', { name: 'Review' }).click();
	await expect(page).toHaveTitle('Review · Waypoint');
	await expect(nav.getByRole('link', { name: 'Review' })).toHaveAttribute('aria-current', 'page');
	await expect(page.getByRole('tab', { name: /^Queue/ })).toHaveAttribute('aria-selected', 'true');
	const item = page.getByRole('list', { name: 'Review queue' }).locator('li').filter({ hasText: message });
	await expect(item).toBeVisible({ timeout: 15_000 });
	await expect(item.getByText('draft_follow_up')).toBeVisible();
	await expect(item.getByText('Needs approval')).toBeVisible();
	await expect(item.getByLabel('Permission: external')).toBeVisible();
	await expect(item.getByRole('link', { name: `Run #${runId}`, exact: true })).toBeVisible();

	// The note step already ran, so the note is on the timeline.
	const events = await page.request.get(`/api/applications/${applicationId}/events`);
	expect((await events.json()).some((event: { body?: string }) => event.body === note)).toBe(true);
	await page.screenshot({ path: `${screenshots}/review-queue.png`, fullPage: true });

	await item.getByRole('button', { name: 'Approve' }).click();
	await expect(page.getByText(`Run #${runId} continues.`)).toBeVisible();
	await expect(item).toHaveCount(0);

	// The action log undoes the note in one click.
	await page.getByRole('tab', { name: 'Action log' }).click();
	const entry = page
		.getByRole('list', { name: 'Action log' })
		.locator('li')
		.filter({ has: page.getByRole('link', { name: `Run #${runId}`, exact: true }) });
	await expect(entry).toHaveCount(1);
	await expect(entry.getByText('add_note')).toBeVisible();
	await entry.getByRole('button', { name: 'Undo' }).click();
	await expect(entry.getByText('Undone', { exact: true })).toBeVisible();
	await expect(entry.getByRole('button', { name: 'Undo' })).toHaveCount(0);
	await page.screenshot({ path: `${screenshots}/review-action-log.png`, fullPage: true });

	// The run finished with the approved draft.
	await entry.getByRole('link', { name: `Run #${runId}`, exact: true }).click();
	await expect(page).toHaveURL(new RegExp(`/runs/${runId}$`));
	await expect(page.getByText('tidy_application').first()).toBeVisible();
	await expect(page.getByText('Succeeded').first()).toBeVisible({ timeout: 15_000 });
	await expect(page.getByText(`${message} (to ${company})`).first()).toBeVisible();
	await page.screenshot({ path: `${screenshots}/review-run-inspector.png`, fullPage: true });

	// Undo removed the note from the timeline.
	await page.goto(`/applications/${applicationId}`);
	await expect(page.getByRole('heading', { name: company })).toBeVisible();
	await expect(page.getByText(note)).toHaveCount(0);

	expect(errors).toEqual([]);
});

test('review: dismiss a low-confidence result and reject an action', async ({ page }) => {
	const errors = collectErrors(page);
	await page.emulateMedia({ reducedMotion: 'reduce' });
	const stamp = Date.now();
	const note = `Unsure note ${stamp}`;
	const message = `Unsure follow-up ${stamp}`;

	await setKillSwitch(page.request, false);
	const applicationId = await createApplication(page.request, `E2E Review ${stamp}`);
	const runId = await startRun(page, applicationId, { note, message, confidence: '0.5' });

	await page.goto('/review');
	const queue = page.getByRole('list', { name: 'Review queue' });
	const result = queue.locator('li').filter({ hasText: note });
	const action = queue.locator('li').filter({ hasText: message });
	await expect(action).toBeVisible({ timeout: 15_000 });
	await expect(result).toBeVisible();
	await expect(result.getByText('Low confidence')).toBeVisible();
	await expect(result.getByText('tidy.note')).toBeVisible();
	await expect(result.getByText('50% confident')).toBeVisible();
	await expect(action.getByText('Needs approval')).toBeVisible();

	// The paused-agents notice follows the kill switch.
	const notice = page.getByText('Agents are paused.', { exact: false });
	await expect(notice).toHaveCount(0);
	try {
		await setKillSwitch(page.request, true);
		await expect(notice).toBeVisible();
		await expect(page.getByRole('status').getByRole('link', { name: 'Dashboard' })).toHaveAttribute(
			'href',
			'/'
		);
		await page.screenshot({ path: `${screenshots}/review-paused.png`, fullPage: true });
	} finally {
		await setKillSwitch(page.request, false);
	}
	await expect(notice).toHaveCount(0);

	await result.getByRole('button', { name: 'Dismiss' }).click();
	await expect(page.getByText('Dismissed', { exact: true })).toBeVisible();
	await expect(result).toHaveCount(0);

	await action.getByRole('button', { name: 'Reject' }).click();
	const dialog = page.getByRole('dialog', { name: 'Reject this action?' });
	await expect(dialog).toBeVisible();
	await page.screenshot({ path: `${screenshots}/review-reject-dialog.png`, fullPage: true });
	await dialog.getByRole('button', { name: 'Reject' }).click();
	await expect(page.getByText(`Run #${runId} ended.`)).toBeVisible();
	await expect(action).toHaveCount(0);

	await page.goto(`/runs/${runId}`);
	await expect(page.getByText('Rejected').first()).toBeVisible();

	expect(errors).toEqual([]);
});
