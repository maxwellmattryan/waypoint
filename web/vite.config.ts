import tailwindcss from '@tailwindcss/vite';
import { loadEnv } from 'vite';
import { defineConfig } from 'vitest/config';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { API_PREFIX, apiTarget } from './src/lib/config.ts';

// VITE_* variables live in the repo-root .env shared with the API and Compose.
const envDir = '..';

export default defineConfig(({ mode }) => {
	const env = loadEnv(mode, envDir, 'VITE_');
	return {
		envDir,
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
			strictPort: true,
			// Vite's default CORS answers any localhost/127.0.0.1 origin on any port, which would let
			// a page on another local port read proxied API responses (job `last_error`, the event
			// stream). With it off, only pages on this origin can read `/api/*`.
			cors: false,
			proxy: {
				// Same-origin API: the browser calls `/api/*` here and the dev server forwards it to the
				// API with the prefix stripped. `Host`, `Origin` and `Sec-Fetch-Site` pass through
				// unchanged, so the API's LocalHost and same-origin guards see the browser's values.
				[API_PREFIX]: {
					target: apiTarget(env),
					changeOrigin: false,
					ws: false,
					rewrite: (path: string) => path.replace(/^\/api/, '')
				}
			}
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
	};
});
