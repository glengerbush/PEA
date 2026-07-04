import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig, loadEnv } from 'vite';

// The dev server is a convenience only (the shipping app serves the SPA over
// pea://). Vite loads .env itself via loadEnv, so no dotenv dependency.
export default defineConfig(({ mode }) => {
	const env = loadEnv(mode, process.cwd(), '');
	return {
		plugins: [tailwindcss(), sveltekit()],
		server: {
			port: Number(env.PORT_FRONTEND) || 3000,
			proxy: {
				'/api': {
					target: `http://localhost:${env.PORT_BACKEND || 4000}`,
					changeOrigin: true,
					rewrite: (path) => path.replace(/^\/api/, ''),
				},
			},
		},
	};
});
