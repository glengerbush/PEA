import path from 'path';
import { pathToFileURL } from 'url';
import { existsSync } from 'fs';
import type { Server } from 'http';
import { createServer } from './api/server';
import { runMigrations } from './database/migrate';
import { closeDb } from './database';
import { startQueue, stopQueue, registerSyncSchedule } from './jobs/queue';
import { startWorkers } from './workers';
import { runShutdownHooks } from './lifecycle';
import { logger } from './config/logger';

// tsc compiles this package to CommonJS, which would rewrite `import()` into
// `require()` and break loading the ESM frontend handler — hence this escape hatch.
const dynamicImport = new Function('specifier', 'return import(specifier)') as (
	specifier: string
) => Promise<any>;

export interface StartOptions {
	port?: number;
	/** Directory of the built SvelteKit frontend (adapter-node output). */
	frontendDir?: string;
}

/**
 * Single-process entrypoint: runs migrations, starts the API, the three queue
 * consumers, the continuous-sync schedule, and serves the built frontend —
 * all on one port, with coordinated shutdown.
 */
export async function startApp(options: StartOptions = {}): Promise<Server> {
	const port =
		options.port ??
		(process.env.PORT_BACKEND ? parseInt(process.env.PORT_BACKEND, 10) : 4000);

	// 1. Migrations first — the retry inside doubles as the wait-for-database gate.
	await runMigrations();

	// 2. Job queue (in-process, backed by the jobs table in archive.db).
	await startQueue();

	// 3. API (awaits i18next and the Meilisearch index config internally).
	const app = await createServer([]);

	// 4. In-process queue consumers + the continuous-sync cron registration.
	await startWorkers();
	await registerSyncSchedule();

	// 4. Serve the built frontend from the same process/port. Mounted after the
	// API routes so /v1 and /api/v1 win; everything else falls through to SvelteKit.
	const frontendDir =
		options.frontendDir ??
		process.env.FRONTEND_BUILD_DIR ??
		path.resolve(__dirname, '../../frontend/build');
	const handlerPath = path.join(frontendDir, 'handler.js');
	if (existsSync(handlerPath)) {
		// adapter-node's handler is ESM; this package is CJS — load it dynamically.
		const { handler } = await dynamicImport(pathToFileURL(handlerPath).href);
		app.use(handler);
		logger.info({ frontendDir }, 'Frontend mounted');
	} else {
		logger.warn(
			{ handlerPath },
			'Frontend build not found — running API-only (dev mode serves the UI via vite dev)'
		);
	}

	const server = app.listen(port, () => {
		logger.info({}, `✅ Open Archiver running on port ${port}`);
	});

	// 5. Coordinated shutdown. Every step is individually caught AND bounded by
	// a timeout so a hung step (e.g. keep-alive sockets pinning server.close)
	// can never prevent the later steps — the shutdown hooks stop the embedded
	// Postgres child and MUST always run, or the child is orphaned.
	let shuttingDown = false;
	// console (synchronous fd writes) instead of pino here: the async transport
	// loses messages written just before process exit.
	// NOTE: the timer is deliberately ref'd — pending promises alone do not keep
	// the event loop alive, and once the http/queue/db handles are closed Node
	// would otherwise exit silently mid-shutdown, orphaning the hooks' children.
	const step = (label: string, ms: number, fn: () => Promise<unknown>): Promise<void> =>
		new Promise<void>((resolve) => {
			const timer = setTimeout(() => {
				console.error(`[shutdown] ${label}: timed out after ${ms}ms, continuing`);
				resolve();
			}, ms);
			fn()
				.then(
					() => console.log(`[shutdown] ${label}: done`),
					(error) => console.error(`[shutdown] ${label}: failed`, error)
				)
				.finally(() => {
					clearTimeout(timer);
					resolve();
				});
		});
	const shutdown = async (signal: string) => {
		if (shuttingDown) {
			return;
		}
		shuttingDown = true;
		console.log(`[shutdown] received ${signal}`);
		await step('http', 5_000, () => {
			const closed = new Promise<void>((resolve) => server.close(() => resolve()));
			// Keep-alive connections would otherwise hold close() open indefinitely.
			(server as any).closeAllConnections?.();
			return closed;
		});
		await step('queue', 26_000, stopQueue); // graceful: waits for in-flight jobs
		await step('db', 5_000, closeDb);
		// External resources owned by the entrypoint (e.g. embedded Postgres).
		await step('hooks', 15_000, () =>
			runShutdownHooks((label, error) =>
				console.error(`[shutdown] hook ${label} failed`, error)
			)
		);
		console.log('[shutdown] complete');
		process.exit(0);
	};
	process.on('SIGINT', () => void shutdown('SIGINT'));
	process.on('SIGTERM', () => void shutdown('SIGTERM'));

	return server;
}
