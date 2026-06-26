#!/usr/bin/env node

import { performance } from 'node:perf_hooks';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

const DEFAULT_BASE_URL = 'http://localhost:3000/api/v1';
const DEFAULT_QUERIES = ['invoice', 'meeting', 'attachment'];

function readNumber(name, fallback) {
	const value = process.env[name];
	if (!value) return fallback;
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function readBoolean(name, fallback = false) {
	const value = process.env[name];
	if (!value) return fallback;
	return ['1', 'true', 'yes', 'on'].includes(value.toLowerCase());
}

function percentile(values, p) {
	if (values.length === 0) return 0;
	const sorted = [...values].sort((a, b) => a - b);
	const index = Math.ceil((p / 100) * sorted.length) - 1;
	return sorted[Math.max(0, Math.min(sorted.length - 1, index))];
}

function summarize(samples) {
	const successes = samples.filter((sample) => sample.ok);
	const durations = successes.map((sample) => sample.ms);
	const total = durations.reduce((sum, value) => sum + value, 0);

	return {
		runs: samples.length,
		failures: samples.length - successes.length,
		averageMs: durations.length ? Number((total / durations.length).toFixed(2)) : 0,
		medianMs: Number(percentile(durations, 50).toFixed(2)),
		p95Ms: Number(percentile(durations, 95).toFixed(2)),
		minMs: durations.length ? Number(Math.min(...durations).toFixed(2)) : 0,
		maxMs: durations.length ? Number(Math.max(...durations).toFixed(2)) : 0,
	};
}

function formatSummary(name, summary) {
	return [
		`${name}`,
		`  runs: ${summary.runs}`,
		`  failures: ${summary.failures}`,
		`  avg: ${summary.averageMs} ms`,
		`  median: ${summary.medianMs} ms`,
		`  p95: ${summary.p95Ms} ms`,
		`  min/max: ${summary.minMs}/${summary.maxMs} ms`,
	].join('\n');
}

async function requestJson(baseUrl, path, token, options = {}) {
	const headers = new Headers(options.headers);
	headers.set('Accept', 'application/json');
	if (token) headers.set('Authorization', `Bearer ${token}`);
	if (options.body && !headers.has('Content-Type')) {
		headers.set('Content-Type', 'application/json');
	}

	const response = await fetch(`${baseUrl}${path}`, {
		...options,
		headers,
	});
	const text = await response.text();
	const body = text ? JSON.parse(text) : null;

	if (!response.ok) {
		const message = body?.message || `${response.status} ${response.statusText}`;
		throw new Error(message);
	}

	return body;
}

async function resolveToken(baseUrl) {
	if (process.env.OPEN_ARCHIVER_TOKEN) {
		return process.env.OPEN_ARCHIVER_TOKEN;
	}

	const email = process.env.OPEN_ARCHIVER_EMAIL;
	const password = process.env.OPEN_ARCHIVER_PASSWORD;
	if (!email || !password) {
		throw new Error(
			'Set OPEN_ARCHIVER_TOKEN or both OPEN_ARCHIVER_EMAIL and OPEN_ARCHIVER_PASSWORD.'
		);
	}

	const login = await requestJson(baseUrl, '/auth/login', null, {
		method: 'POST',
		body: JSON.stringify({ email, password }),
	});

	if (!login?.accessToken) {
		throw new Error('Login succeeded but no accessToken was returned.');
	}

	return login.accessToken;
}

async function discoverArchiveSample(baseUrl, token) {
	try {
		const result = await requestJson(
			baseUrl,
			'/archived-emails?page=1&limit=1&sort=sentAt&direction=desc&includeHiddenDuplicates=true',
			token
		);
		const firstHit = Array.isArray(result?.hits) ? result.hits[0] : null;

		return {
			total: Number(result?.total || 0),
			firstEmailId: firstHit?.id || null,
			firstEmailHasAttachments: Boolean(firstHit?.hasAttachments),
			firstEmailRemoteContentStatus: firstHit?.remoteContentStatus || null,
		};
	} catch (error) {
		return {
			total: null,
			firstEmailId: null,
			firstEmailHasAttachments: false,
			firstEmailRemoteContentStatus: null,
			error: error instanceof Error ? error.message : String(error),
		};
	}
}

async function resolveSourceId(baseUrl, token) {
	if (process.env.OPEN_ARCHIVER_SOURCE_ID) {
		return process.env.OPEN_ARCHIVER_SOURCE_ID;
	}

	const sources = await requestJson(baseUrl, '/ingestion-sources', token);
	if (!Array.isArray(sources) || sources.length === 0) {
		return null;
	}

	return sources[0].id;
}

async function timeRequest(label, fn) {
	const start = performance.now();
	try {
		await fn();
		return {
			label,
			ok: true,
			ms: performance.now() - start,
		};
	} catch (error) {
		return {
			label,
			ok: false,
			ms: performance.now() - start,
			error: error instanceof Error ? error.message : String(error),
		};
	}
}

async function runSamples({ repetitions, warmups, endpoints }) {
	const results = {};

	for (const endpoint of endpoints) {
		for (let i = 0; i < warmups; i += 1) {
			await timeRequest(endpoint.name, endpoint.run);
		}

		const samples = [];
		for (let i = 0; i < repetitions; i += 1) {
			samples.push(await timeRequest(endpoint.name, endpoint.run));
		}
		results[endpoint.name] = {
			summary: summarize(samples),
			samples,
		};
	}

	return results;
}

async function main() {
	const baseUrl = (process.env.OPEN_ARCHIVER_BASE_URL || DEFAULT_BASE_URL).replace(/\/$/, '');
	const repetitions = readNumber('OPEN_ARCHIVER_BENCH_REPETITIONS', 5);
	const warmups = readNumber('OPEN_ARCHIVER_BENCH_WARMUPS', 1);
	const limit = readNumber('OPEN_ARCHIVER_BENCH_LIMIT', 25);
	const includeMutations = readBoolean('OPEN_ARCHIVER_BENCH_INCLUDE_MUTATIONS', false);
	const fuzzyScanBatchSize = readNumber('OPEN_ARCHIVER_BENCH_FUZZY_SCAN_BATCH_SIZE', 100);
	const queryList = process.env.OPEN_ARCHIVER_BENCH_QUERIES
		? process.env.OPEN_ARCHIVER_BENCH_QUERIES.split(',')
				.map((query) => query.trim())
				.filter(Boolean)
		: DEFAULT_QUERIES;

	const token = await resolveToken(baseUrl);
	const sourceId = await resolveSourceId(baseUrl, token);
	const archiveSample = await discoverArchiveSample(baseUrl, token);
	const remoteEmailId =
		process.env.OPEN_ARCHIVER_BENCH_REMOTE_EMAIL_ID || archiveSample.firstEmailId;
	const tagFilter = process.env.OPEN_ARCHIVER_BENCH_TAG;
	const localFolderPathFilter = process.env.OPEN_ARCHIVER_BENCH_LOCAL_FOLDER_PATH;
	const skipped = [];

	const endpoints = [
		{
			name: 'archive-query:first-page',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails?page=1&limit=${limit}&sort=sentAt&direction=desc`,
					token
				),
		},
		{
			name: 'archive-query:attachments-filter',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails?hasAttachments=true&page=1&limit=${limit}&sort=sentAt&direction=desc`,
					token
				),
		},
		...queryList.map((query) => ({
			name: `archive-query-search:${query}`,
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails?q=${encodeURIComponent(query)}&page=1&limit=${limit}&matchingStrategy=last`,
					token
				),
		})),
		{
			name: 'duplicate-review:exact-groups',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails/duplicates/exact?page=1&limit=${limit}`,
					token
				),
		},
		{
			name: 'duplicate-review:fuzzy-groups',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails/duplicates/fuzzy?page=1&limit=${limit}`,
					token
				),
		},
	];

	if (sourceId) {
		endpoints.splice(1, 0, {
			name: 'archive-query:source-filter',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails?ingestionSourceId=${encodeURIComponent(sourceId)}&page=1&limit=${limit}&sort=sentAt&direction=desc`,
					token
				),
		});
	}

	if (tagFilter) {
		endpoints.push({
			name: 'archive-query:tag-filter',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails?tags=${encodeURIComponent(tagFilter)}&page=1&limit=${limit}`,
					token
				),
		});
	}

	if (localFolderPathFilter) {
		endpoints.push({
			name: 'archive-query:local-folder-filter',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails?localFolderPath=${encodeURIComponent(localFolderPathFilter)}&page=1&limit=${limit}`,
					token
				),
		});
	}

	if (remoteEmailId) {
		endpoints.push({
			name: 'remote-content:preview',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails/${encodeURIComponent(remoteEmailId)}/preview`,
					token
				),
		});
	} else {
		skipped.push({
			name: 'remote-content:preview',
			reason: 'No archived email ID found. Set OPEN_ARCHIVER_BENCH_REMOTE_EMAIL_ID to force one.',
		});
	}

	if (includeMutations && remoteEmailId) {
		endpoints.push({
			name: 'remote-content:enqueue',
			run: () =>
				requestJson(
					baseUrl,
					`/archived-emails/${encodeURIComponent(remoteEmailId)}/remote-content/archive`,
					token,
					{ method: 'POST' }
				),
		});
	} else {
		skipped.push({
			name: 'remote-content:enqueue',
			reason: includeMutations
				? 'No archived email ID found. Set OPEN_ARCHIVER_BENCH_REMOTE_EMAIL_ID to force one.'
				: 'Mutation benchmark disabled. Set OPEN_ARCHIVER_BENCH_INCLUDE_MUTATIONS=true to enqueue jobs.',
		});
	}

	if (includeMutations) {
		endpoints.push({
			name: 'duplicate-review:fuzzy-scan-enqueue',
			run: () =>
				requestJson(baseUrl, '/archived-emails/duplicates/fuzzy/scan', token, {
					method: 'POST',
					body: JSON.stringify({ batchSize: fuzzyScanBatchSize }),
				}),
		});
	} else {
		skipped.push({
			name: 'duplicate-review:fuzzy-scan-enqueue',
			reason: 'Mutation benchmark disabled. Set OPEN_ARCHIVER_BENCH_INCLUDE_MUTATIONS=true to enqueue jobs.',
		});
	}

	const startedAt = new Date().toISOString();
	const results = await runSamples({ repetitions, warmups, endpoints });
	const report = {
		startedAt,
		baseUrl,
		sourceId,
		archiveSample,
		repetitions,
		warmups,
		limit,
		queries: queryList,
		includeMutations,
		fuzzyScanBatchSize,
		tagFilter: tagFilter || null,
		localFolderPathFilter: localFolderPathFilter || null,
		remoteEmailId: remoteEmailId || null,
		results,
		skipped,
	};

	console.log(`OpenArchiver baseline benchmark (${startedAt})`);
	console.log(`Base URL: ${baseUrl}`);
	console.log(`Source ID: ${sourceId || 'none'}`);
	console.log(`Archive total: ${archiveSample.total ?? 'unknown'}`);
	console.log(`Remote email ID: ${remoteEmailId || 'none'}`);
	console.log(`Mutation benchmarks: ${includeMutations ? 'enabled' : 'disabled'}`);
	console.log(`Repetitions: ${repetitions}, warmups: ${warmups}, limit: ${limit}`);
	console.log('');

	for (const [name, result] of Object.entries(results)) {
		console.log(formatSummary(name, result.summary));
		const failures = result.samples.filter((sample) => !sample.ok);
		for (const failure of failures) {
			console.log(`  failure: ${failure.error}`);
		}
		console.log('');
	}

	if (skipped.length > 0) {
		console.log('Skipped');
		for (const item of skipped) {
			console.log(`  ${item.name}: ${item.reason}`);
		}
		console.log('');
	}

	if (process.env.OPEN_ARCHIVER_BENCH_OUTPUT) {
		const outputPath = resolve(process.env.OPEN_ARCHIVER_BENCH_OUTPUT);
		await mkdir(dirname(outputPath), { recursive: true });
		await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);
		console.log(`Wrote JSON report to ${outputPath}`);
	}
}

main().catch((error) => {
	console.error(error instanceof Error ? error.message : error);
	process.exitCode = 1;
});
