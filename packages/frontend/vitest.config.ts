import { defineConfig } from 'vitest/config';

// Unit tests for framework-agnostic logic in src/lib (pure .ts modules). These
// run in a plain node environment and deliberately do NOT load the SvelteKit
// plugin, so they stay fast and don't need a browser/DOM. Component-level tests
// would need their own environment and are out of scope here.
export default defineConfig({
	test: {
		environment: 'node',
		include: ['src/lib/**/*.test.ts'],
	},
});
