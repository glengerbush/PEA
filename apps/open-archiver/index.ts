import * as dotenv from 'dotenv';

dotenv.config();

/**
 * Entrypoint for both deployment modes:
 *  - Server/Docker: DATABASE_URL etc. come from the environment.
 *  - Embedded (OA_EMBEDDED=1): a supervised local Postgres + app-data dir are
 *    provisioned first, THEN the app is loaded — the app's config modules
 *    capture env at import time, so the import below must stay lazy.
 */
async function main() {
	const embedded = process.env.OA_EMBEDDED === '1' || process.env.OA_EMBEDDED === 'true';
	if (embedded) {
		// Only pulls node builtins — safe to import before env is complete.
		const { prepareEmbeddedEnvironment } = await import('@open-archiver/backend/embedded');
		await prepareEmbeddedEnvironment();
	}
	const { startApp } = await import('@open-archiver/backend');
	await startApp();
}

main().catch((error) => {
	console.error('Failed to start Open Archiver:', error);
	process.exit(1);
});
