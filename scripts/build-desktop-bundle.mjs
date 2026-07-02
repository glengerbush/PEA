#!/usr/bin/env node
/**
 * Assembles the desktop bundle inputs for Tauri:
 * apps/desktop/src-tauri/resources/   backend bundle + assets + better-sqlite3
 *                                       frontend build + meilisearch + postgres
 *   apps/desktop/src-tauri/binaries/    node runtime as externalBin (per-triple)
 *
 * Run AFTER `pnpm build:oss`. Used both locally and by the release workflow.
 */
import { execFileSync } from 'child_process';
import {
	cpSync, existsSync, mkdirSync, rmSync, chmodSync, statSync,
	readdirSync,
} from 'fs';
import { createRequire } from 'module';


import path from 'path';
import { fileURLToPath } from 'url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tauriDir = path.join(repo, 'apps/desktop/src-tauri');
const resources = path.join(tauriDir, 'resources');
const binaries = path.join(tauriDir, 'binaries');

const log = (msg) => console.log(`[bundle] ${msg}`);

const targetTriple = () => {
	if (process.env.OA_TARGET_TRIPLE) return process.env.OA_TARGET_TRIPLE;
	const map = {
		'linux/x64': 'x86_64-unknown-linux-gnu',
		'linux/arm64': 'aarch64-unknown-linux-gnu',
		'darwin/x64': 'x86_64-apple-darwin',
		'darwin/arm64': 'aarch64-apple-darwin',
	};
	const key = `${process.platform}/${process.arch}`;
	if (!map[key]) throw new Error(`Unsupported platform ${key}`);
	return map[key];
};

// ---------- 0. clean staging ----------
rmSync(resources, { recursive: true, force: true });
rmSync(binaries, { recursive: true, force: true });
mkdirSync(resources, { recursive: true });
mkdirSync(binaries, { recursive: true });

// ---------- 1. backend -> single CJS file ----------
{
	const entry = path.join(repo, 'apps/open-archiver/dist/index.js');
	if (!existsSync(entry)) throw new Error('Run `pnpm build:oss` first (apps dist missing).');
	const rootRequire = createRequire(path.join(repo, 'package.json'));
	const esbuild = rootRequire('esbuild');
	// Resolve the CJS build of i18next-fs-backend from the backend's own
	// dependency tree (pnpm keeps transitives out of the root node_modules).
	const backendRequire = createRequire(path.join(repo, 'packages/backend/package.json'));
	const i18nextFsCjs = backendRequire.resolve('i18next-fs-backend/cjs');
	const result = esbuild.buildSync({
		entryPoints: [entry],
		outfile: path.join(resources, 'backend/index.js'),
		bundle: true,
		platform: 'node',
		target: 'node22',
		format: 'cjs',
		// Its ESM build has top-level await; the CJS build bundles fine.
		alias: { 'i18next-fs-backend': i18nextFsCjs },
		// Native module — shipped as a real package next to the bundle (below).
		external: ['better-sqlite3'],
		logLevel: 'warning',
	});
	if (result.errors?.length) throw new Error('esbuild failed');
	const size = (statSync(path.join(resources, 'backend/index.js')).size / 1e6).toFixed(1);
	log(`backend bundled -> resources/backend/index.js (${size} MB)`);
}

// ---------- 2. static assets ----------
cpSync(path.join(repo, 'packages/backend/src/locales'), path.join(resources, 'locales'), {
	recursive: true,
});
cpSync(
	path.join(repo, 'packages/backend/src/database/migrations'),
	path.join(resources, 'backend/migrations'),
	{ recursive: true }
);
cpSync(path.join(repo, 'packages/frontend/build'), path.join(resources, 'frontend'), {
	recursive: true,
});
// adapter-node's output is NOT self-contained: server chunks import runtime
// deps (clsx, sveltekit-i18n, ...) expecting node_modules beside the build.
// Bundle the SSR handler into a real single file. Entry stays at the ORIGINAL
// location so pnpm can resolve those deps; output replaces the copied handler
// (same directory depth, so its relative client/prerendered lookups still work).
{
	const rootRequire = createRequire(path.join(repo, 'package.json'));
	const esbuild = rootRequire('esbuild');
	const result = esbuild.buildSync({
		entryPoints: [path.join(repo, 'packages/frontend/build/handler.js')],
		outfile: path.join(resources, 'frontend/handler.js'),
		bundle: true,
		platform: 'node',
		format: 'esm',
		target: 'node22',
		allowOverwrite: true,
		// CJS deps bundled into ESM sometimes emit bare `require` calls.
		banner: {
			js: "import { createRequire as __cr } from 'node:module'; const require = __cr(import.meta.url);",
		},
		logLevel: 'warning',
	});
	if (result.errors?.length) throw new Error('esbuild (frontend handler) failed');
	// the unbundled chunks are superseded by the bundled handler
	rmSync(path.join(resources, 'frontend/server'), { recursive: true, force: true });
	rmSync(path.join(resources, 'frontend/index.js'), { force: true });
	log('frontend SSR handler bundled to a single file');
}
log('locales, migrations, frontend copied');

// ---------- 3. better-sqlite3 (native module, external to the bundle) ----------
{
	// Ship the package as a minimal node_modules next to the bundle so the
	// bundle's require('better-sqlite3') resolves normally. Runtime deps:
	// bindings -> file-uri-to-path (tiny, pure JS).
	const pnpmDir = path.join(repo, 'node_modules/.pnpm');
	const findPkg = (name) => {
		const prefix = `${name}@`;
		const hit = readdirSync(pnpmDir).find((entry) => entry.startsWith(prefix));
		if (!hit) throw new Error(`${name} not found in pnpm store`);
		return path.join(pnpmDir, hit, 'node_modules', name);
	};
	const dest = path.join(resources, 'backend/node_modules');
	const sqlitePkg = findPkg('better-sqlite3');
	if (!existsSync(path.join(sqlitePkg, 'build/Release/better_sqlite3.node'))) {
		throw new Error(
			'better-sqlite3 native binding missing — run its install script first (pnpm rebuild better-sqlite3)'
		);
	}
	// better-sqlite3 selectively (its package dir includes ~90MB of SQLite
	// sources); the tiny pure-JS deps are copied whole (bindings' main is
	// bindings.js — selective lists rot, whole-dir doesn't).
	for (const sub of ['package.json', 'lib', 'build/Release/better_sqlite3.node']) {
		cpSync(path.join(sqlitePkg, sub), path.join(dest, 'better-sqlite3', sub), {
			recursive: true,
			dereference: true,
		});
	}
	for (const pkg of ['bindings', 'file-uri-to-path']) {
		cpSync(findPkg(pkg), path.join(dest, pkg), { recursive: true, dereference: true });
	}
	log('better-sqlite3 (+binding) -> resources/backend/node_modules/');
}

// ---------- 5. node runtime as externalBin ----------
{
	const triple = targetTriple();
	const dest = path.join(binaries, `node-${triple}`);
	const source = process.env.OA_BUNDLE_NODE || process.execPath;
	cpSync(source, dest);
	chmodSync(dest, 0o755);
	log(`node runtime (${source}) -> binaries/node-${triple}`);
}

// ---------- summary ----------
const du = (dir) => {
	let total = 0;
	const walk = (d) => {
		for (const e of execFileSync('find', [d, '-type', 'f']).toString().trim().split('\n')) {
			if (e) total += statSync(e).size;
		}
	};
	walk(dir);
	return (total / 1e6).toFixed(0);
};
log(`resources: ${du(resources)} MB, binaries: ${du(binaries)} MB`);
