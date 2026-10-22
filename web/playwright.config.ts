import { fileURLToPath } from 'node:url';
import { defineConfig, devices } from '@playwright/test';

// End-to-end tests against the real API (with a fake `claude`) and the Vite dev server.
// Needs `docker compose up -d --wait db`, a repo-root .env, Chromium
// (`npx --prefix web playwright install chromium`) and ports 8000 and 5173 free.
const fakeClaude = fileURLToPath(new URL('./e2e/fake-claude.sh', import.meta.url));

export default defineConfig({
	testDir: 'e2e',
	fullyParallel: false,
	workers: 1,
	use: { baseURL: 'http://127.0.0.1:5173', trace: 'retain-on-failure' },
	projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
	webServer: [
		{
			command: 'cargo run -p waypoint-api',
			cwd: '..',
			url: 'http://127.0.0.1:8000/health',
			timeout: 300_000,
			reuseExistingServer: false,
			env: {
				WAYPOINT_CLAUDE_BIN: fakeClaude,
				// dotenvy never overrides a variable that is already set, and the API treats empty
				// values as missing, so Google is always "Not configured" here.
				WAYPOINT_GOOGLE_CLIENT_ID: '',
				WAYPOINT_GOOGLE_CLIENT_SECRET: '',
				WAYPOINT_TOKEN_KEY: ''
			}
		},
		{
			command: 'npm run dev',
			url: 'http://127.0.0.1:5173',
			reuseExistingServer: false
		}
	]
});
