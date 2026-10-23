import { expect, test, type APIRequestContext, type Page } from '@playwright/test';

// The memories page through the real UI and API. The Compose database is shared and persists
// between runs, so the test works on its own uniquely worded memory, finds it by that text, never
// asserts on totals, and deletes it at the end even when a step fails. Screenshots go to
// $WAYPOINT_SCREENSHOTS when set (Forge points it at the job's folder).
const screenshots = process.env.WAYPOINT_SCREENSHOTS ?? 'test-results/screenshots';

function collectErrors(page: Page): string[] {
	const errors: string[] = [];
	page.on('console', (message) => {
		if (message.type() === 'error') errors.push(message.text());
	});
	page.on('pageerror', (error) => errors.push(error.message));
	return errors;
}

/** Deletes every memory whose content contains `marker`. */
async function cleanUp(request: APIRequestContext, marker: string) {
	const response = await request.get('/api/memories', { params: { q: marker } });
	if (response.status() !== 200) return;
	for (const memory of (await response.json()) as { id: number; content: string }[]) {
		if (memory.content.includes(marker)) await request.delete(`/api/memories/${memory.id}`);
	}
}

test('memories: add, find, filter by tag, edit and delete', async ({ page }) => {
	const errors = collectErrors(page);
	// No transitions, so screenshots never catch an item mid-fade.
	await page.emulateMedia({ reducedMotion: 'reduce' });
	const stamp = String(Date.now());
	const content = `Remote first, distributed team ${stamp}`;
	const edited = `Remote only, any time zone ${stamp}`;
	const tag = `e2e ${stamp}`;

	try {
		await page.goto('/memories');
		await expect(page).toHaveTitle('Memory · Waypoint');
		const nav = page.getByRole('navigation', { name: 'Main' });
		await expect(nav.getByRole('link', { name: 'Memory' })).toHaveAttribute('aria-current', 'page');
		await expect(page.getByRole('heading', { name: 'What Waypoint knows about me' })).toBeVisible();

		// Add a memory with tags.
		await page.getByRole('button', { name: 'Add memory' }).first().click();
		const dialog = page.getByRole('dialog', { name: 'Add memory' });
		await dialog.getByLabel('Kind').selectOption('preference');
		await dialog.getByLabel('Topic').selectOption('location');
		await dialog.getByLabel('Content').fill(content);
		const tags = dialog.getByLabel('Tags');
		await tags.fill(` E2E   ${stamp} `);
		await tags.press('Enter');
		await tags.fill('Remote');
		await tags.press('Enter');
		await expect(dialog.getByRole('button', { name: `Remove tag ${tag}` })).toBeVisible();
		await page.screenshot({ path: `${screenshots}/memories-dialog.png`, fullPage: true });
		await dialog.getByRole('button', { name: 'Save' }).click();
		await expect(dialog).toBeHidden();
		await expect(page.getByText('Memory saved')).toBeVisible();

		// It shows under Preferences, added by the user.
		const preferences = page.getByRole('list', { name: 'Preferences' });
		const item = preferences.locator('li').filter({ hasText: content });
		await expect(item).toHaveCount(1);
		await expect(item.getByText('Location', { exact: true })).toBeVisible();
		await expect(item.getByText('Added by you')).toBeVisible();
		await expect(item.getByRole('button', { name: tag, exact: true })).toBeVisible();
		await expect(item.getByText('Proposed')).toHaveCount(0);
		await page.screenshot({ path: `${screenshots}/memories-list.png`, fullPage: true });

		// A link to it (as the run inspector makes) scrolls to, focuses and highlights it.
		const found = await page.request.get('/api/memories', { params: { q: stamp } });
		const [{ id }] = (await found.json()) as { id: number }[];
		await page.goto(`/memories?memory=${id}`);
		const card = page.locator(`#memory-${id}`);
		await expect(card).toBeFocused();
		await expect(card).toHaveAttribute('data-highlighted', 'true');
		await expect(card).toBeInViewport();
		await page.screenshot({ path: `${screenshots}/memories-highlight.png`, fullPage: true });
		await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
		await page.screenshot({ path: `${screenshots}/memories-highlight-dark.png`, fullPage: true });
		await page.evaluate(() => document.documentElement.removeAttribute('data-theme'));

		// An id that isn't listed says so; a non-numeric value is ignored.
		await page.goto('/memories?memory=999999999999');
		await expect(page.getByText('That memory no longer exists')).toBeVisible();
		await page.goto('/memories?memory=abc');
		await expect(item).toHaveCount(1);
		await expect(page.locator('[data-highlighted]')).toHaveCount(0);
		await expect(page.getByText('That memory no longer exists')).toHaveCount(0);

		// Search finds it.
		const search = page.getByRole('searchbox', { name: 'Search memories' });
		await search.fill(stamp);
		await expect(item).toHaveCount(1);
		await search.fill('');

		// Clicking its tag filters by that tag; removing the filter tag clears it.
		await item.getByRole('button', { name: tag, exact: true }).click();
		const filterTag = page.getByRole('button', { name: `Remove tag ${tag}` });
		await expect(filterTag).toBeVisible();
		await expect(item).toHaveCount(1);
		await filterTag.click();
		await expect(filterTag).toHaveCount(0);

		// Edit it.
		await item.getByRole('button', { name: 'Edit' }).click();
		const editDialog = page.getByRole('dialog', { name: 'Edit memory' });
		await expect(editDialog.getByLabel('Content')).toHaveValue(content);
		await editDialog.getByLabel('Content').fill(edited);
		await editDialog.getByRole('button', { name: 'Save' }).click();
		await expect(editDialog).toBeHidden();
		const editedItem = preferences.locator('li').filter({ hasText: edited });
		await expect(editedItem).toHaveCount(1);
		await expect(editedItem.getByText('Added by you')).toBeVisible();

		// A search that matches nothing shows the no-match state.
		await search.fill(`zzqx${stamp}nomatch`);
		await expect(page.getByText('No memories match')).toBeVisible();
		await page.screenshot({ path: `${screenshots}/memories-no-match.png`, fullPage: true });
		await page.getByRole('button', { name: 'Clear filters' }).click();
		await expect(search).toHaveValue('');
		await expect(editedItem).toHaveCount(1);

		// Delete it through the confirm dialog.
		await editedItem.getByRole('button', { name: 'Delete' }).click();
		const confirm = page.getByRole('dialog', { name: 'Delete memory' });
		await expect(confirm.getByText("Waypoint will forget this. This can't be undone.")).toBeVisible();
		await confirm.getByRole('button', { name: 'Delete' }).click();
		await expect(confirm).toBeHidden();
		await expect(page.getByText('Memory deleted')).toBeVisible();
		await expect(editedItem).toHaveCount(0);
		const gone = await page.request.get('/api/memories', { params: { q: stamp } });
		expect(await gone.json()).toEqual([]);

		expect(errors).toEqual([]);
	} finally {
		await cleanUp(page.request, stamp);
	}
});
