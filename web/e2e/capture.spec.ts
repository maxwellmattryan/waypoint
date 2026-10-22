import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { expect, test } from '@playwright/test';

// Capture from a local job page with JSON-LD, then add the prefilled application. The
// Compose database is shared, so the test uses a uniquely named company.
const company = `Capture Co ${Date.now()}`;
let server: Server;
let pageUrl: string;

test.beforeAll(async () => {
	const posting = {
		'@context': 'https://schema.org',
		'@type': 'JobPosting',
		title: 'Platform Engineer',
		hiringOrganization: { '@type': 'Organization', name: company },
		jobLocation: {
			'@type': 'Place',
			address: { addressLocality: 'Lisbon', addressCountry: 'PT' }
		},
		baseSalary: {
			'@type': 'MonetaryAmount',
			currency: 'EUR',
			value: { '@type': 'QuantitativeValue', minValue: 70000, maxValue: 90000, unitText: 'YEAR' }
		}
	};
	const html = `<!doctype html><html><head><title>Platform Engineer</title>
<script type="application/ld+json">${JSON.stringify(posting)}</script></head>
<body><h1>Platform Engineer</h1></body></html>`;
	server = createServer((_request, response) => {
		response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
		response.end(html);
	});
	await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
	const { port } = server.address() as AddressInfo;
	pageUrl = `http://127.0.0.1:${port}/jobs/platform-engineer`;
});

test.afterAll(async () => {
	await new Promise<void>((resolve) => server.close(() => resolve()));
});

test('capture from URL prefills the add form', async ({ page }) => {
	const errors: string[] = [];
	page.on('console', (message) => {
		if (message.type() === 'error') errors.push(message.text());
	});
	page.on('pageerror', (error) => errors.push(error.message));

	await page.goto('/applications');
	await page.getByRole('button', { name: 'Capture from URL' }).click();
	const capture = page.getByRole('dialog', { name: 'Capture from URL' });
	await expect(capture.getByLabel('Job posting URL')).toBeFocused();
	await capture.getByLabel('Job posting URL').fill(pageUrl);
	await capture.getByRole('button', { name: 'Capture' }).click();

	const add = page.getByRole('dialog', { name: 'Add application' });
	await expect(add).toBeVisible();
	await expect(capture).toHaveCount(0);
	await expect(
		add.getByText("Filled from the page's job data. Check the details, then add the application.")
	).toBeVisible();
	await expect(add.getByLabel('Company')).toHaveValue(company);
	await expect(add.getByLabel('Title')).toHaveValue('Platform Engineer');
	await expect(add.getByLabel('Location')).toHaveValue('Lisbon, PT');
	await expect(add.getByLabel('Compensation')).toHaveValue('EUR 70,000–90,000 per year');
	await expect(add.getByLabel('Job posting URL')).toHaveValue(pageUrl);

	await add.getByRole('button', { name: 'Add application' }).click();
	await expect(add).toHaveCount(0);
	await expect(page.getByText('Application added')).toBeVisible();
	await expect(page.getByRole('link', { name: new RegExp(company) })).toBeVisible();

	expect(errors).toEqual([]);
});
