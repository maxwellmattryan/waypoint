import { expect, test } from '@playwright/test';

// The Compose database is shared and persists between runs, so the test works on its own
// uniquely named application and never asserts on totals.
test('applications: create, view, status, edit, delete', async ({ page }) => {
	const errors: string[] = [];
	page.on('console', (message) => {
		if (message.type() === 'error') errors.push(message.text());
	});
	page.on('pageerror', (error) => errors.push(error.message));

	const company = `E2E Co ${Date.now()}`;

	await page.goto('/');
	const nav = page.getByRole('navigation', { name: 'Main' });
	await expect(nav.getByRole('link', { name: 'Dashboard' })).toHaveAttribute(
		'aria-current',
		'page'
	);
	await nav.getByRole('link', { name: 'Applications' }).click();
	await expect(page).toHaveURL(/\/applications$/);
	await expect(page).toHaveTitle('Applications · Waypoint');
	await expect(nav.getByRole('link', { name: 'Applications' })).toHaveAttribute(
		'aria-current',
		'page'
	);

	// Add: a blank submit is refused on the client, then a real one goes through.
	await page.getByRole('button', { name: 'Add application' }).first().click();
	const add = page.getByRole('dialog', { name: 'Add application' });
	await expect(add.getByLabel('Company')).toBeFocused();
	await add.getByRole('button', { name: 'Add application' }).click();
	await expect(add.getByText('Required')).toHaveCount(2);
	await add.getByLabel('Company').fill(company);
	await add.getByLabel('Title').fill('Staff Engineer');
	await add.getByLabel('Location').fill('Berlin');
	await add.getByLabel('Job posting URL').fill('https://example.com/jobs/1');
	await add.getByLabel('Notes').fill('First line\nSecond line');
	await add.getByRole('button', { name: 'Add application' }).click();
	await expect(add).toHaveCount(0);
	await expect(page.getByText('Application added')).toBeVisible();

	const row = page.getByRole('link', { name: new RegExp(company) });
	await expect(row).toBeVisible();
	await expect(row).toContainText('Sourced');
	await row.click();

	await expect(page.getByRole('heading', { level: 1, name: company })).toBeVisible();
	await expect(page).toHaveTitle(`${company} · Waypoint`);
	await expect(nav.getByRole('link', { name: 'Applications' })).toHaveAttribute(
		'aria-current',
		'page'
	);
	await expect(page.getByText('First line\nSecond line')).toBeVisible();
	await expect(page.getByRole('link', { name: 'https://example.com/jobs/1' })).toHaveAttribute(
		'target',
		'_blank'
	);

	// Status.
	await page.getByLabel('Status').selectOption('applied');
	await expect(page.getByText('Status set to Applied')).toBeVisible();
	await expect(page.locator('header [data-status]')).toHaveText('Applied');

	// Edit.
	await page.getByRole('button', { name: 'Edit' }).click();
	const edit = page.getByRole('dialog', { name: 'Edit application' });
	await expect(edit.getByLabel('Status')).toHaveCount(0);
	await expect(edit.getByLabel('Company')).toHaveValue(company);
	await edit.getByLabel('Location').fill('Remote');
	await edit.getByRole('button', { name: 'Save' }).click();
	await expect(edit).toHaveCount(0);
	await expect(page.getByText('Application saved')).toBeVisible();
	await expect(page.getByText('Remote', { exact: true })).toBeVisible();
	await expect(page.locator('header [data-status]')).toHaveText('Applied');

	// Stages: a blank name is refused on the client, then two go in (Enter and the button).
	const stages = page.getByRole('region', { name: 'Stages' });
	const newStage = stages.getByLabel('New stage');
	const stageRows = stages.getByRole('listitem');
	await expect(stages.getByText('No stages yet')).toBeVisible();
	await stages.getByRole('button', { name: 'Add stage' }).click();
	await expect(stages.getByText('Required')).toBeVisible();
	await newStage.fill('Recruiter screen');
	await newStage.press('Enter');
	await expect(stageRows).toHaveText(['Recruiter screen']);
	await expect(newStage).toHaveValue('');
	await newStage.fill('Onsite');
	await stages.getByRole('button', { name: 'Add stage' }).click();
	await expect(stageRows).toHaveText(['Recruiter screen', 'Onsite']);
	await expect(stageRows.first().getByRole('button', { name: 'Move up' })).toBeDisabled();
	await expect(stageRows.last().getByRole('button', { name: 'Move down' })).toBeDisabled();

	await stages.getByRole('checkbox', { name: 'Recruiter screen' }).check();
	await expect(stages.getByRole('checkbox', { name: 'Recruiter screen' })).toBeChecked();
	await expect(stages.getByRole('checkbox', { name: 'Recruiter screen' })).toBeEnabled();
	await stageRows.filter({ hasText: 'Onsite' }).getByRole('button', { name: 'Move up' }).click();
	await expect(stageRows).toHaveText(['Onsite', 'Recruiter screen']);
	await stageRows
		.filter({ hasText: 'Recruiter screen' })
		.getByRole('button', { name: 'Remove stage' })
		.click();
	await expect(stageRows).toHaveText(['Onsite']);

	// A note.
	const timeline = page.getByRole('region', { name: 'Timeline' });
	const timelineItems = timeline.getByRole('listitem');
	await expect(timeline.getByRole('button', { name: 'Add note' })).toBeDisabled();
	await timeline.getByLabel('Add a note').fill('Recruiter will\ncall back');
	await timeline.getByRole('button', { name: 'Add note' }).click();
	await expect(timeline.getByLabel('Add a note')).toHaveValue('');
	await expect(timelineItems.first()).toContainText('call back');

	// Rejected goes through the dialog; cancelling it changes and sends nothing.
	const rejection = page.getByRole('dialog', { name: 'Mark as rejected?' });
	await page.getByLabel('Status').selectOption('rejected');
	await expect(rejection).toContainText(`${company} · Staff Engineer`);
	await rejection.getByRole('button', { name: 'Cancel' }).click();
	await expect(rejection).toHaveCount(0);
	await expect(page.getByLabel('Status')).toHaveValue('applied');
	await expect(page.locator('header [data-status]')).toHaveText('Applied');
	await expect(timelineItems.first()).toContainText('Note');

	await page.getByLabel('Status').selectOption('rejected');
	await rejection.getByLabel('Feedback (optional)').fill('Went with someone more senior');
	await rejection.getByRole('button', { name: 'Mark rejected' }).click();
	await expect(rejection).toHaveCount(0);
	await expect(page.getByText('Status set to Rejected')).toBeVisible();
	await expect(page.locator('header [data-status]')).toHaveText('Rejected');
	await expect(timelineItems).toHaveText([
		/^\s*Rejection feedback.*Went with someone more senior/s,
		/^\s*Status changed from Applied to Rejected/,
		/^\s*Note.*Recruiter will\s+call back/s,
		/^\s*Stage removed: Recruiter screen/,
		/^\s*Stage completed: Recruiter screen/,
		/^\s*Stage added: Onsite/,
		/^\s*Stage added: Recruiter screen/,
		/^\s*Status changed from Sourced to Applied/,
		/^\s*Added as Sourced/
	]);

	// Once rejected, feedback can be added on its own; it is required there.
	await timeline.getByRole('button', { name: 'Add rejection feedback' }).click();
	const feedbackOnly = page.getByRole('dialog', { name: 'Add rejection feedback' });
	await expect(feedbackOnly.getByRole('button', { name: 'Save feedback' })).toBeDisabled();
	await feedbackOnly.getByLabel('Feedback').fill('Reapply next year');
	await feedbackOnly.getByRole('button', { name: 'Save feedback' }).click();
	await expect(feedbackOnly).toHaveCount(0);
	await expect(timelineItems.first()).toContainText('Reapply next year');

	// Delete.
	await page.getByRole('button', { name: 'Delete' }).click();
	const confirm = page.getByRole('dialog', { name: 'Delete application?' });
	await expect(confirm).toContainText(`${company} · Staff Engineer will be removed permanently.`);
	await confirm.getByRole('button', { name: 'Delete' }).click();
	await expect(page).toHaveURL(/\/applications$/);
	await expect(page.getByText('Application deleted')).toBeVisible();
	await expect(page.getByRole('heading', { name: 'Applications' })).toBeVisible();
	await expect(page.getByRole('link', { name: new RegExp(company) })).toHaveCount(0);

	expect(errors).toEqual([]);
});

test('applications: an unknown or malformed id shows not found', async ({ page }) => {
	for (const id of ['999999999', 'abc']) {
		await page.goto(`/applications/${id}`);
		await expect(page.getByText('Application not found')).toBeVisible();
	}
	await page.getByRole('button', { name: 'Back to applications' }).click();
	await expect(page).toHaveURL(/\/applications$/);
});
