import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	// Consult https://svelte.dev/docs/kit/integrations
	// for more information about preprocessors
	preprocess: vitePreprocess(),
	kit: {
		// Static SPA: the local engine serves these files and falls back to
		// index.html for client-side routing (ssr/prerender are off in the
		// root layout).
		adapter: adapter({
			fallback: 'index.html',
		}),
	},
};

export default config;
