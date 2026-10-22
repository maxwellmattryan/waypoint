import { expect, test, type Page } from '@playwright/test';

function collectErrors(page: Page, errors: string[]) {
	page.on('console', (message) => {
		if (message.type() === 'error') errors.push(message.text());
	});
	page.on('pageerror', (error) => errors.push(error.message));
}

// The Compose database is shared and persists between runs, so the test works on its own
// uniquely named application and never asserts on totals.
test('board: drag, menu move and live updates', async ({ page, context }) => {
	const errors: string[] = [];
	collectErrors(page, errors);

	const company = `E2E Board ${Date.now()}`;
	const name = `${company} · Staff Engineer`;
	const column = (p: Page, status: string) => p.getByRole('region', { name: status });
	const cardLink = (p: Page, status: string) => column(p, status).getByRole('link', { name });

	await page.goto('/applications');
	await page.getByRole('button', { name: 'Add application' }).first().click();
	const add = page.getByRole('dialog', { name: 'Add application' });
	await add.getByLabel('Company').fill(company);
	await add.getByLabel('Title').fill('Staff Engineer');
	await add.getByRole('button', { name: 'Add application' }).click();
	await expect(add).toHaveCount(0);

	// Switch to Board: the URL mirrors it and the new card is in Sourced.
	await page.getByRole('tab', { name: 'Board' }).click();
	await expect(page).toHaveURL(/\/applications\?view=board$/);
	await expect(cardLink(page, 'Sourced')).toHaveCount(1);

	// Drag it to Applied.
	const card = column(page, 'Sourced').locator('[draggable="true"]', {
		has: page.getByRole('link', { name })
	});
	await card.dragTo(column(page, 'Applied').locator('[data-drop-zone]'));
	await expect(cardLink(page, 'Applied')).toHaveCount(1);
	await expect(cardLink(page, 'Sourced')).toHaveCount(0);
	await expect(page.getByText('Moved to Applied')).toBeVisible();

	// A reload keeps the Board view and the new column.
	await page.reload();
	await expect(page).toHaveURL(/\/applications\?view=board$/);
	await expect(page.getByRole('tab', { name: 'Board' })).toHaveAttribute('aria-selected', 'true');
	await expect(cardLink(page, 'Applied')).toHaveCount(1);

	// A second page on the board follows along without a reload.
	const other = await context.newPage();
	collectErrors(other, errors);
	await other.goto('/applications?view=board');
	await expect(cardLink(other, 'Applied')).toHaveCount(1);

	// Move it to Interviewing from its Move menu by keyboard.
	await column(page, 'Applied').getByRole('button', { name: `Move ${name}` }).focus();
	await page.keyboard.press('Enter');
	const menu = page.getByRole('menu', { name: 'Move to' });
	await expect(menu.getByRole('menuitem', { name: 'Sourced' })).toBeFocused();
	await page.keyboard.press('ArrowDown');
	await expect(menu.getByRole('menuitem', { name: 'Interviewing' })).toBeFocused();
	await page.keyboard.press('Enter');
	await expect(page.getByText('Moved to Interviewing')).toBeVisible();
	const moved = column(page, 'Interviewing').getByRole('button', { name: `Move ${name}` });
	await expect(moved).toHaveCount(1);
	await expect(moved).toBeFocused();

	await expect(cardLink(other, 'Interviewing')).toHaveCount(1);
	await expect(cardLink(other, 'Applied')).toHaveCount(0);

	// The List view reads the same live state.
	await page.getByRole('tab', { name: 'List' }).click();
	await expect(page).toHaveURL(/\/applications$/);
	const row = page.getByRole('link', { name: new RegExp(company) });
	await expect(row).toContainText('Interviewing');

	// Delete it from the detail page; the second page drops the card.
	await row.click();
	await expect(page.getByRole('heading', { level: 1, name: company })).toBeVisible();
	await page.getByRole('button', { name: 'Delete' }).click();
	await page
		.getByRole('dialog', { name: 'Delete application?' })
		.getByRole('button', { name: 'Delete' })
		.click();
	await expect(page.getByText('Application deleted')).toBeVisible();
	await expect(other.getByRole('link', { name })).toHaveCount(0);

	expect(errors).toEqual([]);
});
