#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const args = new Set(process.argv.slice(2));
const withTika = args.has('--with-tika');
const scriptDir = dirname(fileURLToPath(import.meta.url));
const setupScript = resolve(scriptDir, 'setup-local-env.mjs');
const setupArgs = [setupScript, '--if-missing'];

if (withTika) {
	setupArgs.push('--with-tika');
}

const setup = spawnSync(process.execPath, setupArgs, {
	cwd: process.cwd(),
	stdio: 'inherit',
});

if (setup.error) {
	console.error(`Failed to prepare local environment: ${setup.error.message}`);
	process.exit(1);
}

if (setup.status !== 0) {
	process.exit(setup.status ?? 1);
}

const composeArgs = withTika
	? ['compose', '--profile', 'tika', 'up', '-d']
	: ['compose', 'up', '-d'];
const compose = spawnSync('docker', composeArgs, {
	cwd: process.cwd(),
	stdio: 'inherit',
});

if (compose.error) {
	console.error(`Failed to start Docker Compose: ${compose.error.message}`);
	process.exit(1);
}

process.exit(compose.status ?? 0);
