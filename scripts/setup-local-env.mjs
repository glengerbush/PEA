#!/usr/bin/env node

import { existsSync, readFileSync, writeFileSync, chmodSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { resolve } from 'node:path';

const args = new Set(process.argv.slice(2));
const force = args.has('--force');
const withTika = args.has('--with-tika');
const envPath = resolve(process.cwd(), '.env');
const examplePath = resolve(process.cwd(), '.env.example');

if (existsSync(envPath) && !force) {
	console.error('.env already exists. Re-run with --force to replace it.');
	process.exit(1);
}

if (!existsSync(examplePath)) {
	console.error('Could not find .env.example in the current directory.');
	process.exit(1);
}

const template = readFileSync(examplePath, 'utf8');
const values = new Map();

for (const line of template.split(/\r?\n/)) {
	const match = line.match(/^([A-Z0-9_]+)=(.*)$/);
	if (match) {
		values.set(match[1], match[2]);
	}
}

const portFrontend = stripQuotes(values.get('PORT_FRONTEND') || '3000') || '3000';
const postgresDb = stripQuotes(values.get('POSTGRES_DB') || 'open_archive') || 'open_archive';
const postgresUser = stripQuotes(values.get('POSTGRES_USER') || 'admin') || 'admin';
const postgresPassword = randomSecret(32);
const localUrl = `http://127.0.0.1:${portFrontend}`;

const generatedValues = new Map([
	['NODE_ENV', 'production'],
	['PERSONAL_MODE', 'true'],
	['COMPOSE_PROFILES', withTika ? 'tika' : ''],
	['APP_URL', localUrl],
	['ORIGIN', localUrl],
	['POSTGRES_PASSWORD', postgresPassword],
	[
		'DATABASE_URL',
		`postgresql://${postgresUser}:${postgresPassword}@postgres:5432/${postgresDb}`,
	],
	['MEILI_MASTER_KEY', randomSecret(32)],
	['REDIS_PASSWORD', randomSecret(32)],
	['REDIS_USER', ''],
	['STORAGE_ENCRYPTION_KEY', randomHex(32)],
	['JWT_SECRET', randomSecret(64)],
	['ENCRYPTION_KEY', randomHex(32)],
	['TIKA_URL', withTika ? 'http://tika:9998' : ''],
]);

const rendered = template
	.split(/\r?\n/)
	.map((line) => {
		const match = line.match(/^([A-Z0-9_]+)=/);
		if (!match) {
			return line;
		}

		const key = match[1];
		if (!generatedValues.has(key)) {
			return line;
		}

		return `${key}=${generatedValues.get(key)}`;
	})
	.join('\n');

writeFileSync(envPath, rendered.endsWith('\n') ? rendered : `${rendered}\n`, { mode: 0o600 });
chmodSync(envPath, 0o600);

console.log('Created .env with local-only defaults and generated secrets.');
console.log(`Frontend URL: ${localUrl}`);
console.log(
	withTika
		? 'Tika enabled. Start with: docker compose up -d'
		: 'Tika disabled by default. Re-run with --force --with-tika before starting to enable it.'
);

function randomSecret(byteLength) {
	return randomBytes(byteLength).toString('base64url');
}

function randomHex(byteLength) {
	return randomBytes(byteLength).toString('hex');
}

function stripQuotes(value) {
	return value.replace(/^['"]|['"]$/g, '');
}
