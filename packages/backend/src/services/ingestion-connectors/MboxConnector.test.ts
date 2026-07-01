import { afterEach, describe, expect, it, vi } from 'vitest';
import { access, mkdir, mkdtemp, rm, writeFile } from 'fs/promises';
import { tmpdir } from 'os';
import { join } from 'path';
import type { EmailObject } from '@open-archiver/types';

const createEmailMessage = (id: string, subject: string) => `Message-ID: <${id}@example.local>
From: ${id} <${id}@example.local>
To: archive <archive@example.local>
Subject: ${subject}
Date: Fri, 01 Jan 2021 00:00:00 +0000

Hello from ${subject}.
`;

const createMboxMessage = (id: string, subject: string) =>
	`From ${id}@example.local Fri Jan 01 00:00:00 2021
${createEmailMessage(id, subject)}`;

const createEmlxMessage = (id: string, subject: string) => {
	const email = Buffer.from(createEmailMessage(id, subject));
	return Buffer.concat([
		Buffer.from(`${email.length}\n`),
		email,
		Buffer.from('<?xml version="1.0"?><plist><dict></dict></plist>'),
	]);
};

describe('MboxConnector', () => {
	let tempDirs: string[] = [];

	afterEach(async () => {
		vi.unstubAllEnvs();
		vi.resetModules();
		await Promise.all(tempDirs.map((dir) => rm(dir, { recursive: true, force: true })));
		tempDirs = [];
	});

	it('imports .mbox files recursively from a local folder', async () => {
		const importRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-import-'));
		const storageRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-storage-'));
		tempDirs.push(importRoot, storageRoot);

		const nestedFolder = join(importRoot, 'Clients');
		await mkdir(nestedFolder, { recursive: true });
		await writeFile(join(importRoot, 'Inbox.mbox'), createMboxMessage('inbox', 'Inbox'));
		await writeFile(join(nestedFolder, 'Acme.mbox'), createMboxMessage('acme', 'Acme'));
		await writeFile(join(nestedFolder, 'notes.txt'), 'not an mbox');

		vi.stubEnv('STORAGE_TYPE', 'local');
		vi.stubEnv('STORAGE_LOCAL_ROOT_PATH', storageRoot);
		vi.stubEnv('LOG_LEVEL', 'silent');
		vi.resetModules();

		const { MboxConnector } = await import('./MboxConnector');
		const connector = new MboxConnector({
			type: 'mbox_import',
			localFilePath: importRoot,
		});
		const emails: EmailObject[] = [];

		await expect(connector.testConnection()).resolves.toBe(true);

		try {
			for await (const email of connector.fetchEmails('archive@example.local')) {
				if (email) {
					emails.push(email);
				}
			}

			expect(emails).toHaveLength(2);
			expect(emails.map((email) => email.path).sort()).toEqual(['Clients/Acme', 'Inbox']);
			expect(emails.map((email) => email.subject).sort()).toEqual(['Acme', 'Inbox']);
		} finally {
			await Promise.all(emails.map((email) => rm(email.tempFilePath, { force: true })));
		}
	});

	it('does not emit an empty email when the first message has no envelope line', async () => {
		const importRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-import-'));
		const storageRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-storage-'));
		tempDirs.push(importRoot, storageRoot);
		const filePath = join(importRoot, 'No-envelope.mbox');
		await writeFile(filePath, createEmailMessage('no-envelope', 'No envelope'));

		vi.stubEnv('STORAGE_TYPE', 'local');
		vi.stubEnv('STORAGE_LOCAL_ROOT_PATH', storageRoot);
		vi.stubEnv('LOG_LEVEL', 'silent');
		vi.resetModules();

		const { MboxConnector } = await import('./MboxConnector');
		const connector = new MboxConnector({
			type: 'mbox_import',
			localFilePath: filePath,
		});
		const emails: EmailObject[] = [];

		try {
			for await (const email of connector.fetchEmails('archive@example.local')) {
				if (email) emails.push(email);
			}

			expect(emails).toHaveLength(1);
			expect(emails[0].subject).toBe('No envelope');
			expect(emails[0].from[0].address).toBe('no-envelope@example.local');
		} finally {
			await Promise.all(emails.map((email) => rm(email.tempFilePath, { force: true })));
		}
	});

	it('imports Apple Mail package directories and ignores duplicate EMLX copies', async () => {
		const importRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-import-'));
		const storageRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-storage-'));
		tempDirs.push(importRoot, storageRoot);

		const rootMessages = join(
			importRoot,
			'Archive.mbox',
			'B85D6E46-FD07-49B1-8D63-ECB6011C2E2F',
			'Data',
			'1',
			'Messages'
		);
		const childMessages = join(
			importRoot,
			'Archive.mbox',
			'Child.mbox',
			'B85D6E46-FD07-49B1-8D63-ECB6011C2E2F',
			'Data',
			'2',
			'Messages'
		);
		const duplicateMessages = join(
			importRoot,
			'Archive.mbox',
			'Child.mbox',
			'32B4B3EE-919A-42BC-B4AC-9F53D6E92131.noindex',
			'Data',
			'3',
			'Messages'
		);
		await Promise.all([
			mkdir(rootMessages, { recursive: true }),
			mkdir(childMessages, { recursive: true }),
			mkdir(duplicateMessages, { recursive: true }),
		]);
		await writeFile(join(rootMessages, '1.emlx'), createEmlxMessage('root', 'Root'));
		const childMessage = createEmlxMessage('child', 'Child');
		await writeFile(join(childMessages, '2.emlx'), childMessage);
		await writeFile(join(duplicateMessages, '3.emlx'), childMessage);

		vi.stubEnv('STORAGE_TYPE', 'local');
		vi.stubEnv('STORAGE_LOCAL_ROOT_PATH', storageRoot);
		vi.stubEnv('LOG_LEVEL', 'silent');
		vi.resetModules();

		const { MboxConnector } = await import('./MboxConnector');
		const connector = new MboxConnector({
			type: 'mbox_import',
			localFilePath: importRoot,
		});
		const emails: EmailObject[] = [];

		await expect(connector.testConnection()).resolves.toBe(true);

		try {
			for await (const email of connector.fetchEmails('archive@example.local')) {
				if (email) emails.push(email);
			}

			expect(emails).toHaveLength(2);
			expect(emails.map((email) => email.path).sort()).toEqual(['Archive', 'Archive/Child']);
			expect(emails.map((email) => email.subject).sort()).toEqual(['Child', 'Root']);
		} finally {
			await Promise.all(emails.map((email) => rm(email.tempFilePath, { force: true })));
		}
	});

	it('imports and cleans up multiple uploaded mbox files', async () => {
		const storageRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-storage-'));
		tempDirs.push(storageRoot);
		const uploadDirectory = join(storageRoot, 'uploads');
		await mkdir(uploadDirectory, { recursive: true });
		await writeFile(join(uploadDirectory, 'one'), createMboxMessage('one', 'One'));
		await writeFile(join(uploadDirectory, 'two'), createMboxMessage('two', 'Two'));

		vi.stubEnv('STORAGE_TYPE', 'local');
		vi.stubEnv('STORAGE_LOCAL_ROOT_PATH', storageRoot);
		vi.stubEnv('LOG_LEVEL', 'silent');
		vi.resetModules();

		const { MboxConnector } = await import('./MboxConnector');
		const connector = new MboxConnector({
			type: 'mbox_import',
			uploadedFiles: [
				{ fileName: 'One.mbox', filePath: 'uploads/one' },
				{ fileName: 'Two.mbox', filePath: 'uploads/two' },
			],
		});
		const emails: EmailObject[] = [];

		await expect(connector.testConnection()).resolves.toBe(true);

		try {
			for await (const email of connector.fetchEmails('archive@example.local')) {
				if (email) emails.push(email);
			}

			expect(emails).toHaveLength(2);
			expect(emails.map((email) => email.path).sort()).toEqual(['One', 'Two']);
			await expect(access(join(uploadDirectory, 'one'))).rejects.toThrow();
			await expect(access(join(uploadDirectory, 'two'))).rejects.toThrow();
		} finally {
			await Promise.all(emails.map((email) => rm(email.tempFilePath, { force: true })));
		}
	});

	it('imports uploaded Apple Mail folders with their relative mailbox paths', async () => {
		const storageRoot = await mkdtemp(join(tmpdir(), 'oa-mbox-storage-'));
		tempDirs.push(storageRoot);
		const uploadDirectory = join(storageRoot, 'uploads');
		await mkdir(uploadDirectory, { recursive: true });
		const rootMessage = createEmlxMessage('root-upload', 'Root upload');
		const childMessage = createEmlxMessage('child-upload', 'Child upload');
		await writeFile(join(uploadDirectory, 'root'), rootMessage);
		await writeFile(join(uploadDirectory, 'child'), childMessage);
		await writeFile(join(uploadDirectory, 'duplicate'), childMessage);

		vi.stubEnv('STORAGE_TYPE', 'local');
		vi.stubEnv('STORAGE_LOCAL_ROOT_PATH', storageRoot);
		vi.stubEnv('LOG_LEVEL', 'silent');
		vi.resetModules();

		const { MboxConnector } = await import('./MboxConnector');
		const connector = new MboxConnector({
			type: 'mbox_import',
			uploadedFiles: [
				{
					fileName: '1.emlx',
					filePath: 'uploads/root',
					relativePath: 'Exports/Archive.mbox/Data/1/Messages/1.emlx',
				},
				{
					fileName: '2.emlx',
					filePath: 'uploads/child',
					relativePath: 'Exports/Archive.mbox/Child.mbox/Data/2/Messages/2.emlx',
				},
				{
					fileName: '3.emlx',
					filePath: 'uploads/duplicate',
					relativePath:
						'Exports/Archive.mbox/Child.mbox/UUID.noindex/Data/3/Messages/3.emlx',
				},
			],
		});
		const emails: EmailObject[] = [];

		await expect(connector.testConnection()).resolves.toBe(true);

		try {
			for await (const email of connector.fetchEmails('archive@example.local')) {
				if (email) emails.push(email);
			}

			expect(emails).toHaveLength(2);
			expect(emails.map((email) => email.path).sort()).toEqual(['Archive', 'Archive/Child']);
			expect(emails.map((email) => email.subject).sort()).toEqual([
				'Child upload',
				'Root upload',
			]);
			await expect(access(join(uploadDirectory, 'root'))).rejects.toThrow();
			await expect(access(join(uploadDirectory, 'child'))).rejects.toThrow();
			await expect(access(join(uploadDirectory, 'duplicate'))).rejects.toThrow();
		} finally {
			await Promise.all(emails.map((email) => rm(email.tempFilePath, { force: true })));
		}
	});
});
