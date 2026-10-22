import { expect, test } from '@playwright/test';

test('dashboard smoke: health, google, test claude', async ({ page }) => {
	const errors: string[] = [];
	page.on('console', (message) => {
		if (message.type() === 'error') errors.push(message.text());
	});
	page.on('pageerror', (error) => errors.push(error.message));

	await page.goto('/');
	await expect(page).toHaveTitle('Waypoint');
	await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible();
	await expect(page.getByRole('status')).toHaveText('Live');

	const system = page.getByRole('region', { name: 'System' });
	await expect(system.getByText('API').locator('xpath=..')).toContainText('OK');
	await expect(system.getByText('Database').locator('xpath=..')).toContainText('OK');

	const google = page.getByRole('region', { name: 'Google' });
	await expect(google.getByText('Not configured')).toBeVisible();
	await expect(google.getByRole('button', { name: 'Connect Google' })).toBeDisabled();

	// The placeholder is gone; applications live on their own page now.
	await expect(page.getByText('No applications yet')).toHaveCount(0);
	await expect(
		page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Applications' })
	).toBeVisible();

	// The job events arrive over SSE through the dev server's /api proxy; no reload.
	const claude = page.getByRole('region', { name: 'Claude' });
	await claude.getByRole('button', { name: 'Test Claude' }).click();
	await expect(claude.getByRole('button', { name: 'Testing…' })).toBeDisabled();
	await expect(claude.getByTestId('claude-status')).toHaveText('Running');
	await expect(claude.getByTestId('claude-status')).toHaveText('Succeeded', { timeout: 15_000 });
	await expect(claude.getByTestId('claude-reply')).toHaveText('pong');
	await expect(claude.getByRole('button', { name: 'Test again' })).toBeEnabled();

	expect(errors).toEqual([]);
});

test('proxy sends no CORS headers to other origins', async ({ request }) => {
	const origins = ['http://localhost:3000', 'http://127.0.0.1:8000', 'https://evil.example'];
	for (const origin of origins) {
		const health = await request.get('/api/health', { headers: { Origin: origin } });
		expect(health.status(), `GET /api/health from ${origin}`).toBe(200);
		expect(health.headers()['access-control-allow-origin'], origin).toBeUndefined();

		const preflight = await request.fetch('/api/dev/jobs/test-prompt', {
			method: 'OPTIONS',
			headers: {
				Origin: origin,
				'Access-Control-Request-Method': 'POST',
				'Access-Control-Request-Headers': 'x-waypoint-dev'
			}
		});
		const headers = Object.keys(preflight.headers());
		expect(
			headers.filter((name) => name.startsWith('access-control-allow-')),
			`OPTIONS preflight from ${origin}`
		).toEqual([]);
	}
});
