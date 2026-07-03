#!/usr/bin/env node
/**
 * Assembles the desktop bundle inputs for Tauri (R4: pure-Rust engine):
 *   apps/desktop/src-tauri/resources/frontend   the static SPA build
 *
 * Everything else — API, job queue, ingestion, search, migrations — is
 * compiled into the desktop binary via the oa-engine library (migrations are
 * embedded with include_dir). No Node runtime, no backend bundle.
 *
 * Run AFTER building the frontend (`pnpm --filter @pea/frontend build`).
 * Used both locally and by the release workflow.
 */
import { execFileSync } from 'child_process';
import { cpSync, existsSync, mkdirSync, rmSync, statSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tauriDir = path.join(repo, 'apps/desktop/src-tauri');
const resources = path.join(tauriDir, 'resources');

const log = (msg) => console.log(`[bundle] ${msg}`);

rmSync(resources, { recursive: true, force: true });
rmSync(path.join(tauriDir, 'binaries'), { recursive: true, force: true }); // pre-R4 leftovers
mkdirSync(resources, { recursive: true });

const frontendBuild = path.join(repo, 'packages/frontend/build');
if (!existsSync(path.join(frontendBuild, 'index.html'))) {
	throw new Error('Frontend build missing — run `pnpm --filter @pea/frontend build` first.');
}
cpSync(frontendBuild, path.join(resources, 'frontend'), { recursive: true });

const du = (dir) => {
	let total = 0;
	for (const e of execFileSync('find', [dir, '-type', 'f']).toString().trim().split('\n')) {
		if (e) total += statSync(e).size;
	}
	return (total / 1e6).toFixed(1);
};
log(`frontend -> resources/frontend (${du(resources)} MB — the whole resource payload)`);
