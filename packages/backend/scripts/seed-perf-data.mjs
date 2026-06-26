#!/usr/bin/env node

import 'dotenv/config';
import { createHash, randomUUID } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { MeiliSearch } from 'meilisearch';
import postgres from 'postgres';

const DEFAULT_EMAIL_COUNT = 2500;
const DEFAULT_BATCH_SIZE = 500;
const DEFAULT_WRITE_EML_COUNT = 25;
const DEFAULT_OUTPUT_PATH = 'tmp/perf-seed.json';
const INDEX_NAME = 'emails';
const SYNTHETIC_SOURCE_PREFIX = 'Synthetic Perf Source';

const SOURCE_FOLDERS = [
	'INBOX',
	'Sent',
	'Receipts',
	'Projects/Atlas',
	'Projects/Beacon',
	'Travel',
	'Archive/2024',
	'Archive/2025',
];

const MAILBOXES = ['work@example.local', 'personal@example.local'];
const QUERY_TERMS = ['invoice', 'meeting', 'attachment'];
const TAG_POOL = ['finance', 'project-atlas', 'travel', 'receipts', 'legal', 'seed'];
const SENDERS = [
	['Acme Billing', 'billing@acme.example'],
	['Nadia Project', 'nadia@projects.example'],
	['Travel Desk', 'travel@example.local'],
	['Legal Notices', 'notices@legal.example'],
	['Ops Robot', 'ops@example.local'],
	['Casey Manager', 'casey@example.local'],
];

function readNumber(name, fallback) {
	const value = process.env[name];
	if (!value) return fallback;
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

function readBoolean(name, fallback = false) {
	const value = process.env[name];
	if (!value) return fallback;
	return ['1', 'true', 'yes', 'on'].includes(value.toLowerCase());
}

function requireEnv(name) {
	const value = process.env[name];
	if (!value) {
		throw new Error(`${name} is required.`);
	}
	return value;
}

function encodeDatabaseUrl(databaseUrl) {
	const url = new URL(databaseUrl);
	if (url.password) {
		url.password = encodeURIComponent(url.password);
	}
	return url.toString();
}

function hash(value) {
	return createHash('sha256').update(value).digest('hex');
}

function pick(values, index) {
	return values[index % values.length];
}

function normalizeSubjectHash(subject) {
	return hash(subject.toLowerCase().replace(/\s+/g, ' ').trim());
}

function buildBody(term, sourcePath, index) {
	const invoiceText =
		'This invoice includes line items, tax, payment terms, and receipt references.';
	const meetingText =
		'This meeting note includes agenda, decisions, follow-up owners, and schedule details.';
	const attachmentText =
		'This attachment memo references filenames, extracted attachment content, and document review.';

	return [
		`Synthetic ${term} archive message ${index}.`,
		term === 'invoice' ? invoiceText : '',
		term === 'meeting' ? meetingText : '',
		term === 'attachment' ? attachmentText : '',
		`Imported from ${sourcePath}.`,
		'Search baseline data should stay compact, deterministic, and quick to rebuild.',
	]
		.filter(Boolean)
		.join(' ');
}

function buildEml({ row, document }) {
	const html = [
		'<!doctype html>',
		'<html>',
		'<body>',
		`<h1>${document.subject}</h1>`,
		`<p>${document.body}</p>`,
		'<img alt="tracking pixel" src="https://assets.example.invalid/open.png">',
		'<img alt="logo" srcset="https://assets.example.invalid/logo-small.png 1x, https://assets.example.invalid/logo-large.png 2x">',
		'</body>',
		'</html>',
	].join('');

	return [
		`From: "${row.sender_name}" <${row.sender_email}>`,
		`To: ${document.to.join(', ')}`,
		`Subject: ${row.subject}`,
		`Message-ID: ${row.message_id_header}`,
		`Date: ${new Date(row.sent_at).toUTCString()}`,
		'MIME-Version: 1.0',
		'Content-Type: text/html; charset=UTF-8',
		'',
		html,
	].join('\r\n');
}

async function waitForTask(client, task) {
	const taskUid = task?.taskUid ?? task?.uid;
	if (typeof taskUid !== 'number') {
		return;
	}
	if (typeof client.waitForTask === 'function') {
		await client.waitForTask(taskUid, { timeOutMs: 120_000, intervalMs: 100 });
		return;
	}
	await client.tasks.waitForTask(taskUid, { timeOutMs: 120_000, intervalMs: 100 });
}

async function ensureEmailIndex(client) {
	try {
		await client.getIndex(INDEX_NAME);
	} catch {
		await waitForTask(client, await client.createIndex(INDEX_NAME, { primaryKey: 'id' }));
	}

	const index = client.index(INDEX_NAME);
	await waitForTask(client, await index.update({ primaryKey: 'id' }));
	await waitForTask(
		client,
		await index.updateSettings({
			searchableAttributes: [
				'subject',
				'body',
				'from',
				'senderName',
				'to',
				'cc',
				'bcc',
				'attachments.filename',
				'attachments.content',
				'userEmail',
				'sourcePath',
				'sourceLabels',
				'localFolderPath',
				'tags',
			],
			filterableAttributes: [
				'from',
				'senderName',
				'to',
				'cc',
				'bcc',
				'timestamp',
				'archivedAt',
				'ingestionSourceId',
				'userEmail',
				'hasAttachments',
				'sourcePath',
				'sourceLabels',
				'localFolderId',
				'localFolderPath',
				'tags',
				'threadId',
				'messageIdHeader',
				'duplicateOfEmailId',
				'duplicateReviewStatus',
				'isDuplicateHidden',
				'sizeBytes',
			],
			sortableAttributes: ['timestamp', 'archivedAt', 'from', 'subject', 'sizeBytes'],
			pagination: {
				maxTotalHits: 1_000_000,
			},
		})
	);

	return index;
}

async function insertJsonRecordset(sql, tableName, rows, columnTypes, columns, batchSize) {
	for (let offset = 0; offset < rows.length; offset += batchSize) {
		const batch = rows.slice(offset, offset + batchSize);
		await sql`
			INSERT INTO ${sql(tableName)} (${sql(columns)})
			SELECT ${sql(columns)}
			FROM jsonb_to_recordset(${sql.json(batch)}) AS x(${sql.unsafe(columnTypes)})
			ON CONFLICT DO NOTHING
		`;
	}
}

async function resetSyntheticData(sql, meiliClient, index) {
	const oldIds = await sql`
		SELECT ae.id
		FROM archived_emails ae
		JOIN ingestion_sources source ON source.id = ae.ingestion_source_id
		WHERE source.name LIKE ${`${SYNTHETIC_SOURCE_PREFIX}%`}
	`;
	const ids = oldIds.map((row) => row.id);

	await sql`
		DELETE FROM fuzzy_duplicate_groups
		WHERE group_key LIKE 'perf:%'
	`;
	await sql`
		DELETE FROM ingestion_sources
		WHERE name LIKE ${`${SYNTHETIC_SOURCE_PREFIX}%`}
	`;
	await sql`
		DELETE FROM archive_folders
		WHERE path LIKE 'Imports/Synthetic Perf Source%'
	`;

	if (ids.length > 0) {
		for (let offset = 0; offset < ids.length; offset += DEFAULT_BATCH_SIZE) {
			await waitForTask(
				meiliClient,
				await index.deleteDocuments(ids.slice(offset, offset + DEFAULT_BATCH_SIZE))
			);
		}
	}
}

async function createFolders(sql, sourceName) {
	const folderByPath = new Map();
	const rootPath = `Imports/${sourceName}`;
	const planned = [
		{ path: rootPath, name: sourceName, parentPath: null },
		...MAILBOXES.flatMap((mailbox) => {
			const mailboxPath = `${rootPath}/${mailbox}`;
			return [
				{ path: mailboxPath, name: mailbox, parentPath: rootPath },
				...SOURCE_FOLDERS.map((folderPath) => {
					const parts = folderPath.split('/');
					return {
						path: `${mailboxPath}/${folderPath}`,
						name: parts[parts.length - 1],
						parentPath:
							parts.length === 1
								? mailboxPath
								: `${mailboxPath}/${parts.slice(0, -1).join('/')}`,
					};
				}),
			];
		}),
	];

	for (const item of planned) {
		const parentId = item.parentPath ? folderByPath.get(item.parentPath) || null : null;
		const id = randomUUID();
		const [folder] = await sql`
			INSERT INTO archive_folders (id, parent_id, name, path)
			VALUES (${id}, ${parentId}, ${item.name}, ${item.path})
			ON CONFLICT (path) DO UPDATE
			SET parent_id = EXCLUDED.parent_id,
				name = EXCLUDED.name,
				updated_at = now()
			RETURNING id, path
		`;
		folderByPath.set(folder.path, folder.id);
	}

	return { rootPath, folderByPath };
}

function buildRows({ count, sourceId, sourceName, folderByPath, rootPath, runId, userEmail }) {
	const rows = [];
	const documents = [];
	const fuzzyGroups = [];
	const fuzzyGroupLinks = [];
	const writtenEmls = [];
	const now = Date.now();

	for (let i = 0; i < count; i += 1) {
		const id = randomUUID();
		const mailbox = pick(MAILBOXES, i);
		const sourcePath = pick(SOURCE_FOLDERS, Math.floor(i / 3) + i);
		const localFolderPath = `${rootPath}/${mailbox}/${sourcePath}`;
		const localFolderId = folderByPath.get(localFolderPath) || null;
		const term = pick(QUERY_TERMS, i);
		const tag = pick(TAG_POOL, i);
		const sender = pick(SENDERS, i);
		const sentAt = new Date(now - i * 47 * 60 * 1000);
		const archivedAt = new Date(now - i * 17 * 60 * 1000);
		const isExactDuplicate = i % 25 === 1;
		const exactBase = isExactDuplicate ? i - 1 : i;
		const exactMessageId = `<perf-${runId}-message-${exactBase}@example.local>`;
		const body = buildBody(term, sourcePath, i);
		const subject = `${term[0].toUpperCase()}${term.slice(1)} ${sourcePath.replace('/', ' ')} ${Math.floor(i / 10)}`;
		const hasAttachments = i % 4 === 0 || term === 'attachment';
		const storagePath = `open-archiver/perf/${runId}/messages/${id}.eml`;
		const storageHash = hash(`storage-${runId}-${exactBase}`);
		const threadId = `perf-thread-${Math.floor(i / 3)}`;
		const duplicateFuzzyGroupKey =
			i % 60 < 3 ? `perf:${runId}:fuzzy:${Math.floor(i / 60)}` : null;
		const recipients = {
			to: [{ name: mailbox, address: mailbox }],
			cc:
				i % 7 === 0
					? [{ name: 'Archive Team', address: 'archive-team@example.local' }]
					: [],
			bcc: [],
		};
		const sourceLabels = [
			sourcePath.split('/')[0],
			mailbox.startsWith('work') ? 'work' : 'personal',
		];
		const tags = Array.from(new Set([tag, 'seed', term === 'invoice' ? 'finance' : tag]));
		const attachmentFingerprint = hasAttachments ? hash(`attachment-${runId}-${i % 25}`) : null;
		const row = {
			id,
			thread_id: threadId,
			ingestion_source_id: sourceId,
			user_email: mailbox,
			message_id_header: exactMessageId,
			provider_message_id: `perf-provider-${runId}-${i}`,
			sent_at: sentAt.toISOString(),
			archived_at: archivedAt.toISOString(),
			subject,
			sender_name: sender[0],
			sender_email: sender[1],
			recipients,
			storage_path: storagePath,
			storage_hash_sha256: storageHash,
			size_bytes: 600 + body.length + (hasAttachments ? 4096 : 0),
			is_indexed: true,
			has_attachments: hasAttachments,
			source_path: sourcePath,
			source_labels: sourceLabels,
			local_folder_id: localFolderId,
			local_folder_path: localFolderPath,
			duplicate_subject_hash: normalizeSubjectHash(subject),
			duplicate_fuzzy_group_key: duplicateFuzzyGroupKey,
			duplicate_body_hash: hash(body.toLowerCase().replace(/\s+/g, ' ').trim()),
			duplicate_recipient_fingerprint: hash(
				[mailbox, ...recipients.cc.map((r) => r.address)].join(',')
			),
			duplicate_attachment_fingerprint: attachmentFingerprint,
			remote_content_status: 'not_started',
			path: sourcePath,
			tags,
		};
		const document = {
			id,
			userEmail: row.user_email,
			from: row.sender_email,
			senderName: row.sender_name,
			to: recipients.to.map((recipient) => recipient.address),
			cc: recipients.cc.map((recipient) => recipient.address),
			bcc: [],
			subject,
			body,
			attachments: hasAttachments
				? [
						{
							filename: `${term}-${i}.pdf`,
							content: `${term} attachment content for synthetic benchmark ${i}`,
						},
					]
				: [],
			timestamp: sentAt.getTime(),
			archivedAt: archivedAt.getTime(),
			ingestionSourceId: sourceId,
			threadId,
			messageIdHeader: row.message_id_header,
			hasAttachments,
			sourcePath,
			sourceLabels,
			localFolderId,
			localFolderPath,
			tags,
			duplicateOfEmailId: null,
			duplicateReviewStatus: 'unique',
			isDuplicateHidden: false,
			sizeBytes: row.size_bytes,
		};

		rows.push(row);
		documents.push(document);

		if (duplicateFuzzyGroupKey) {
			if (i % 60 === 0) {
				const groupId = randomUUID();
				fuzzyGroups.push({
					id: groupId,
					group_key: duplicateFuzzyGroupKey,
					status: 'pending',
					score: 91,
					signals: {
						subject: true,
						sender: true,
						recipients: true,
						sentTime: true,
						bodyHash: true,
						attachments: hasAttachments,
					},
				});
			}
			const group = fuzzyGroups[fuzzyGroups.length - 1];
			fuzzyGroupLinks.push({
				group_id: group.id,
				email_id: id,
				suggested_keeper: i % 60 === 0,
			});
		}

		writtenEmls.push({ row, document });
	}

	return {
		rows,
		documents,
		fuzzyGroups,
		fuzzyGroupLinks,
		writtenEmls,
		sourceName,
		sourceId,
		userEmail,
		runId,
		count,
		firstEmailId: rows[0]?.id || null,
		tagFilter: 'finance',
		localFolderPathFilter: rows[0]?.local_folder_path || null,
	};
}

async function writeEmlFiles(storageRootPath, fixtures, limit) {
	if (!storageRootPath || limit <= 0) {
		return 0;
	}

	const selected = fixtures.slice(0, limit);
	for (const fixture of selected) {
		const fullPath = join(storageRootPath, fixture.row.storage_path);
		await mkdir(dirname(fullPath), { recursive: true });
		await writeFile(fullPath, buildEml(fixture));
	}

	return selected.length;
}

async function main() {
	const databaseUrl = encodeDatabaseUrl(requireEnv('DATABASE_URL'));
	const meiliHost = requireEnv('MEILI_HOST');
	const meiliApiKey = process.env.MEILI_MASTER_KEY || process.env.MEILI_API_KEY;
	const count = readNumber('OPEN_ARCHIVER_SEED_EMAILS', DEFAULT_EMAIL_COUNT);
	const batchSize = readNumber('OPEN_ARCHIVER_SEED_BATCH_SIZE', DEFAULT_BATCH_SIZE);
	const writeEmlCount = readNumber('OPEN_ARCHIVER_SEED_WRITE_EML_COUNT', DEFAULT_WRITE_EML_COUNT);
	const shouldReset = readBoolean('OPEN_ARCHIVER_SEED_RESET', true);
	const runId =
		process.env.OPEN_ARCHIVER_SEED_RUN_ID ||
		`perf-${new Date()
			.toISOString()
			.replace(/[-:.TZ]/g, '')
			.slice(0, 14)}`;
	const sourceName = `${SYNTHETIC_SOURCE_PREFIX} ${runId}`;
	const userEmail = process.env.OPEN_ARCHIVER_SEED_USER_EMAIL || MAILBOXES[0];
	const outputPath = resolve(process.env.OPEN_ARCHIVER_SEED_OUTPUT || DEFAULT_OUTPUT_PATH);
	const storageRootPath =
		process.env.STORAGE_TYPE === 'local' ? process.env.STORAGE_LOCAL_ROOT_PATH : null;

	const sql = postgres(databaseUrl, { max: 1 });
	const meiliClient = new MeiliSearch({
		host: meiliHost,
		apiKey: meiliApiKey,
	});
	const index = await ensureEmailIndex(meiliClient);

	if (shouldReset) {
		await resetSyntheticData(sql, meiliClient, index);
	}

	const [owner] = await sql`
		SELECT id
		FROM users
		ORDER BY created_at ASC
		LIMIT 1
	`;
	const sourceId = randomUUID();
	await sql`
		INSERT INTO ingestion_sources (
			id,
			user_id,
			name,
			provider,
			status,
			sync_state,
			preserve_original_file,
			last_sync_finished_at
		)
		VALUES (
			${sourceId},
			${owner?.id || null},
			${sourceName},
			'generic_imap',
			'imported',
			${sql.json({ runId, count, generatedBy: 'seed-perf-data' })},
			true,
			now()
		)
	`;

	const { rootPath, folderByPath } = await createFolders(sql, sourceName);
	const fixtures = buildRows({
		count,
		sourceId,
		sourceName,
		folderByPath,
		rootPath,
		runId,
		userEmail,
	});

	await insertJsonRecordset(
		sql,
		'archived_emails',
		fixtures.rows,
		[
			'id uuid',
			'thread_id text',
			'ingestion_source_id uuid',
			'user_email text',
			'message_id_header text',
			'provider_message_id text',
			'sent_at timestamptz',
			'archived_at timestamptz',
			'subject text',
			'sender_name text',
			'sender_email text',
			'recipients jsonb',
			'storage_path text',
			'storage_hash_sha256 text',
			'size_bytes bigint',
			'is_indexed boolean',
			'has_attachments boolean',
			'source_path text',
			'source_labels jsonb',
			'local_folder_id uuid',
			'local_folder_path text',
			'duplicate_subject_hash text',
			'duplicate_fuzzy_group_key text',
			'duplicate_body_hash text',
			'duplicate_recipient_fingerprint text',
			'duplicate_attachment_fingerprint text',
			'remote_content_status text',
			'path text',
			'tags jsonb',
		].join(', '),
		[
			'id',
			'thread_id',
			'ingestion_source_id',
			'user_email',
			'message_id_header',
			'provider_message_id',
			'sent_at',
			'archived_at',
			'subject',
			'sender_name',
			'sender_email',
			'recipients',
			'storage_path',
			'storage_hash_sha256',
			'size_bytes',
			'is_indexed',
			'has_attachments',
			'source_path',
			'source_labels',
			'local_folder_id',
			'local_folder_path',
			'duplicate_subject_hash',
			'duplicate_fuzzy_group_key',
			'duplicate_body_hash',
			'duplicate_recipient_fingerprint',
			'duplicate_attachment_fingerprint',
			'remote_content_status',
			'path',
			'tags',
		],
		batchSize
	);

	await insertJsonRecordset(
		sql,
		'fuzzy_duplicate_groups',
		fixtures.fuzzyGroups,
		'id uuid, group_key text, status text, score integer, signals jsonb',
		['id', 'group_key', 'status', 'score', 'signals'],
		batchSize
	);

	await insertJsonRecordset(
		sql,
		'fuzzy_duplicate_group_emails',
		fixtures.fuzzyGroupLinks,
		'group_id uuid, email_id uuid, suggested_keeper boolean',
		['group_id', 'email_id', 'suggested_keeper'],
		batchSize
	);

	for (let offset = 0; offset < fixtures.documents.length; offset += batchSize) {
		const task = await index.addDocuments(fixtures.documents.slice(offset, offset + batchSize));
		await waitForTask(meiliClient, task);
	}

	const writtenEmlCount = await writeEmlFiles(
		storageRootPath,
		fixtures.writtenEmls,
		writeEmlCount
	);

	const report = {
		runId,
		sourceId,
		sourceName,
		emailCount: count,
		fuzzyGroupCount: fixtures.fuzzyGroups.length,
		firstEmailId: fixtures.firstEmailId,
		tagFilter: fixtures.tagFilter,
		localFolderPathFilter: fixtures.localFolderPathFilter,
		storageRootPathConfigured: Boolean(storageRootPath),
		writtenEmlCount,
		benchmarkEnv: {
			OPEN_ARCHIVER_SOURCE_ID: sourceId,
			OPEN_ARCHIVER_BENCH_REMOTE_EMAIL_ID: fixtures.firstEmailId,
			OPEN_ARCHIVER_BENCH_TAG: fixtures.tagFilter,
			OPEN_ARCHIVER_BENCH_LOCAL_FOLDER_PATH: fixtures.localFolderPathFilter,
		},
	};

	await mkdir(dirname(outputPath), { recursive: true });
	await writeFile(outputPath, `${JSON.stringify(report, null, 2)}\n`);

	console.log(`Seeded ${count} synthetic archived emails.`);
	console.log(`Source ID: ${sourceId}`);
	console.log(`Remote preview email ID: ${fixtures.firstEmailId}`);
	console.log(`Tag filter: ${fixtures.tagFilter}`);
	console.log(`Local folder filter: ${fixtures.localFolderPathFilter}`);
	console.log(`Wrote ${writtenEmlCount} EML fixtures for preview tests.`);
	console.log(`Wrote seed report to ${outputPath}`);

	await sql.end({ timeout: 5 });
}

main().catch((error) => {
	console.error(error instanceof Error ? error.stack || error.message : error);
	process.exitCode = 1;
});
