import path from 'path';
import net from 'net';
import { existsSync, mkdirSync, readFileSync, writeFileSync, chmodSync } from 'fs';
import { randomBytes } from 'crypto';
import { resolveDataDir } from './data-dir';

/**
 * Embedded (desktop) mode: everything lives under one app-data directory —
 * SQLite database + FTS5 search index (archive.db) and the encrypted email
 * storage. One process, zero child services.
 *
 * IMPORT-ORDER CONTRACT: this module must be loaded and prepareEmbeddedEnvironment()
 * awaited BEFORE any app module (config/*, database/*, api/*) is required — those
 * capture env at import time. It therefore only imports node builtins and the
 * dependency-free data-dir module.
 */

export const isEmbeddedMode = (): boolean =>
	process.env.OA_EMBEDDED === '1' || process.env.OA_EMBEDDED === 'true';

export { resolveDataDir } from './data-dir';

interface EmbeddedSecrets {
	encryptionKey: string; // hex 32B — encrypts ingestion-source credentials
	storageEncryptionKey: string; // hex 32B — encrypts files at rest
	meiliMasterKey: string; // for the supervised Meilisearch child
}

/**
 * Loads or provisions the persistent secrets. Existing values are NEVER
 * regenerated — these keys encrypt data already on disk; replacing them would
 * make source credentials and stored files unreadable.
 */
const loadOrCreateSecrets = (dataDir: string): EmbeddedSecrets => {
	const file = path.join(dataDir, 'secrets.json');
	let existing: Partial<EmbeddedSecrets> & Record<string, unknown> = {};
	if (existsSync(file)) {
		existing = JSON.parse(readFileSync(file, 'utf8'));
	}
	const secrets = {
		...existing, // preserve unknown keys (e.g. legacy pgPassword) untouched
		encryptionKey: existing.encryptionKey || randomBytes(32).toString('hex'),
		storageEncryptionKey: existing.storageEncryptionKey || randomBytes(32).toString('hex'),
		meiliMasterKey: existing.meiliMasterKey || randomBytes(32).toString('base64url'),
	};
	writeFileSync(file, JSON.stringify(secrets, null, '\t') + '\n');
	chmodSync(file, 0o600);
	return secrets;
};

/** Returns `preferred` if free on 127.0.0.1, otherwise the next free port. */
const findFreePort = async (preferred: number): Promise<number> => {
	for (let port = preferred; port < preferred + 50; port++) {
		const free = await new Promise<boolean>((resolve) => {
			const server = net.createServer();
			server.once('error', () => resolve(false));
			server.listen(port, '127.0.0.1', () => {
				server.close(() => resolve(true));
			});
		});
		if (free) {
			return port;
		}
	}
	throw new Error(`No free port found near ${preferred}`);
};

const setIfUnset = (key: string, value: string): void => {
	if (!process.env[key]) {
		process.env[key] = value;
	}
};

export interface EmbeddedRuntime {
	dataDir: string;
	appPort: number;
}

/**
 * Provisions the app-data dir, secrets, and env, and starts the Meilisearch
 * child. Must complete before the app modules are required.
 */
export const prepareEmbeddedEnvironment = async (): Promise<EmbeddedRuntime> => {
	const dataDir = resolveDataDir();
	const storageDir = path.join(dataDir, 'storage');
	mkdirSync(storageDir, { recursive: true });

	const secrets = loadOrCreateSecrets(dataDir);

	const appPort = process.env.PORT_BACKEND
		? parseInt(process.env.PORT_BACKEND, 10)
		: await findFreePort(47100);

	// Env must be complete before the app modules are required. The SQLite
	// database and FTS5 search index need no env or child processes:
	// database/index.ts derives archive.db from this same data dir.
	setIfUnset('NODE_ENV', 'production');
	setIfUnset('PERSONAL_MODE', 'true');
	process.env.PORT_BACKEND = String(appPort);
	setIfUnset('STORAGE_TYPE', 'local');
	setIfUnset('STORAGE_LOCAL_ROOT_PATH', storageDir);
	setIfUnset('ENCRYPTION_KEY', secrets.encryptionKey);
	setIfUnset('STORAGE_ENCRYPTION_KEY', secrets.storageEncryptionKey);

	// Dead-man switch for the desktop shell (portable complement to Linux's
	// PR_SET_PDEATHSIG): when reparented (parent died), shut down gracefully.
	if (process.env.OA_WATCH_PARENT === '1') {
		const originalParent = process.ppid;
		const watcher = setInterval(() => {
			if (process.ppid !== originalParent) {
				console.log('[embedded] Parent shell exited — shutting down.');
				clearInterval(watcher);
				process.kill(process.pid, 'SIGTERM');
			}
		}, 3000);
		watcher.unref();
	}

	// Breadcrumb for the desktop shell (which window URL to open, what to health-check).
	const runtime: EmbeddedRuntime = { dataDir, appPort };
	writeFileSync(
		path.join(dataDir, 'runtime.json'),
		JSON.stringify({ ...runtime, pid: process.pid }, null, '\t') + '\n'
	);
	console.log(`[embedded] Data dir: ${dataDir} | app port ${appPort}`);
	return runtime;
};
