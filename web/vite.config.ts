import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';

export default defineConfig({
	// Read VITE_* variables from the repo-root .env shared with the API and Compose.
	envDir: '..',
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// Static SPA: every URL falls back to index.html and renders on the client.
			adapter: adapter({ fallback: 'index.html' })
		})
	],
	server: {
		host: '127.0.0.1',
		port: Number(process.env.WAYPOINT_WEB_PORT ?? 5173),
		strictPort: true
	},
	test: {
		expect: { requireAssertions: true },
		projects: [
			{
				// Component tests: mount Svelte components in a jsdom DOM.
				extends: true,
				resolve: { conditions: ['browser'] },
				test: {
					name: 'client',
					environment: 'jsdom',
					include: ['src/**/*.svelte.test.ts'],
					setupFiles: ['./vitest-setup-client.ts']
				}
			},
			{
				// Plain TypeScript tests: no DOM.
				extends: true,
				test: {
					name: 'server',
					environment: 'node',
					include: ['src/**/*.test.ts'],
					exclude: ['src/**/*.svelte.test.ts']
				}
			}
		]
	}
});
