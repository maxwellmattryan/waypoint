import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';

// The same fixture the API tests use: a one-page PDF whose text includes the phrase below.
const fixture = fileURLToPath(
	new URL('../../api/tests/fixtures/resumes/resume.pdf', import.meta.url)
);
const phrase = 'Waypoint Fixture Resume';

// The Compose database is shared and persists between runs, so the test works on its own
// uniquely named resume and application and never asserts on totals.
test('resumes: upload, view, link to an application, unlink, delete', async ({ page }) => {
	const errors: string[] = [];
	page.on('console', (message) => {
		if (message.type() === 'error') errors.push(message.text());
	});
	page.on('pageerror', (error) => errors.push(error.message));

	const name = `E2E Resume ${Date.now()}`;
	const company = `E2E Resume Co ${Date.now()}`;

	await page.goto('/');
	const nav = page.getByRole('navigation', { name: 'Main' });
	await nav.getByRole('link', { name: 'Resumes' }).click();
	await expect(page).toHaveURL(/\/resumes$/);
	await expect(page).toHaveTitle('Resumes · Waypoint');
	await expect(nav.getByRole('link', { name: 'Resumes' })).toHaveAttribute('aria-current', 'page');

	// Upload: the name defaults from the file, then gets a unique label.
	await page.getByRole('button', { name: 'Upload resume' }).first().click();
	const upload = page.getByRole('dialog', { name: 'Upload resume' });
	await upload.getByLabel('PDF file').setInputFiles(fixture);
	await expect(upload.getByLabel('Name')).toHaveValue('resume');
	await expect(upload.getByText('resume.pdf')).toBeVisible();
	await upload.getByLabel('Name').fill(name);
	await upload.getByRole('button', { name: 'Upload' }).click();
	await expect(upload).toHaveCount(0);
	await expect(page.getByText('Resume uploaded')).toBeVisible();

	const row = page.getByRole('link', { name: new RegExp(name) });
	await expect(row).toBeVisible();
	await expect(row).toContainText('resume.pdf');
	await expect(row).not.toContainText('No text');
	await row.click();

	// Viewer.
	await expect(page.getByRole('heading', { level: 1, name })).toBeVisible();
	await expect(page).toHaveTitle(`${name} · Waypoint`);
	await expect(nav.getByRole('link', { name: 'Resumes' })).toHaveAttribute('aria-current', 'page');
	await expect(page.getByText(phrase)).toBeVisible();
	const open = page.getByRole('link', { name: 'Open PDF' });
	await expect(open).toHaveAttribute('target', '_blank');
	await expect(open).toHaveAttribute('href', /\/api\/resumes\/\d+\/pdf$/);
	const resumeUrl = page.url();
	const pdf = await page.request.get((await open.getAttribute('href')) ?? '');
	expect(pdf.status()).toBe(200);
	expect(pdf.headers()['content-type']).toBe('application/pdf');
	await expect(page.getByRole('button', { name: 'Delete' })).toBeEnabled();

	// Link it on an application, and see it after a reload.
	const created = await page.request.post('/api/applications', {
		data: { company, title: 'Staff Engineer' }
	});
	expect(created.status()).toBe(201);
	const applicationId: number = (await created.json()).id;
	await page.goto(`/applications/${applicationId}`);
	const select = page.getByLabel('Resume sent');
	await expect(select).toHaveValue('');
	await select.selectOption({ label: name });
	await expect(page.getByText('Resume set')).toBeVisible();
	await expect(page.getByRole('link', { name, exact: true })).toBeVisible();
	await page.reload();
	await expect(page.getByLabel('Resume sent')).not.toHaveValue('');
	await expect(page.getByRole('link', { name, exact: true })).toHaveAttribute(
		'href',
		new URL(resumeUrl).pathname
	);

	// Delete is blocked while it is linked.
	await page.getByRole('link', { name, exact: true }).click();
	await expect(page.getByRole('heading', { level: 1, name })).toBeVisible();
	await expect(page.getByRole('button', { name: 'Delete' })).toBeDisabled();
	await expect(page.getByText('Used by 1 application; unlink it before deleting.')).toBeVisible();

	// Clear the link, then delete.
	await page.goto(`/applications/${applicationId}`);
	await page.getByLabel('Resume sent').selectOption({ label: 'None' });
	await expect(page.getByText('Resume cleared')).toBeVisible();
	await expect(page.getByRole('link', { name, exact: true })).toHaveCount(0);

	await page.goto(resumeUrl);
	await page.getByRole('button', { name: 'Delete' }).click();
	const confirm = page.getByRole('dialog', { name: 'Delete resume?' });
	await confirm.getByRole('button', { name: 'Delete' }).click();
	await expect(page).toHaveURL(/\/resumes$/);
	await expect(page.getByText('Resume deleted')).toBeVisible();
	await expect(page.getByRole('link', { name: new RegExp(name) })).toHaveCount(0);

	const removed = await page.request.delete(`/api/applications/${applicationId}`);
	expect(removed.status()).toBe(204);
	expect(errors).toEqual([]);
});

test('resumes: an unknown or malformed id shows not found', async ({ page }) => {
	for (const id of ['999999999', 'abc']) {
		await page.goto(`/resumes/${id}`);
		await expect(page.getByText('Resume not found')).toBeVisible();
	}
	await page.getByRole('button', { name: 'Back to resumes' }).click();
	await expect(page).toHaveURL(/\/resumes$/);
});
