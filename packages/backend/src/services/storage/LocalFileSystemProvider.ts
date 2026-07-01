import { IStorageProvider, LocalStorageConfig } from '@open-archiver/types';
import { promises as fs } from 'fs';
import * as path from 'path';
import { createReadStream, createWriteStream } from 'fs';
import { pipeline } from 'stream/promises';

export class LocalFileSystemProvider implements IStorageProvider {
	private readonly rootPath: string;

	constructor(config: LocalStorageConfig) {
		this.rootPath = config.rootPath;
	}

	/**
	 * Resolve a storage key to an absolute path and guarantee it stays inside
	 * the storage root. Rejects any key that escapes via `..` or an absolute
	 * path, so untrusted key segments (attachment/upload filenames, mailbox
	 * folder names) can never traverse the filesystem.
	 */
	private resolveWithinRoot(filePath: string): string {
		const root = path.resolve(this.rootPath);
		const fullPath = path.resolve(root, filePath);
		if (fullPath !== root && !fullPath.startsWith(root + path.sep)) {
			throw new Error('Invalid storage path: resolved path escapes storage root');
		}
		return fullPath;
	}

	async put(filePath: string, content: Buffer | NodeJS.ReadableStream): Promise<void> {
		const fullPath = this.resolveWithinRoot(filePath);
		const dir = path.dirname(fullPath);
		await fs.mkdir(dir, { recursive: true });

		if (Buffer.isBuffer(content)) {
			await fs.writeFile(fullPath, content);
		} else {
			const writeStream = createWriteStream(fullPath);
			await pipeline(content, writeStream);
		}
	}

	async get(filePath: string): Promise<NodeJS.ReadableStream> {
		const fullPath = this.resolveWithinRoot(filePath);
		try {
			await fs.access(fullPath);
			return createReadStream(fullPath);
		} catch (error) {
			throw new Error('File not found');
		}
	}

	async delete(filePath: string): Promise<void> {
		const fullPath = this.resolveWithinRoot(filePath);
		try {
			await fs.rm(fullPath, { recursive: true, force: true });
		} catch (error: any) {
			// Even with force: true, other errors might occur (e.g., permissions)
			if (error.code !== 'ENOENT') {
				throw error;
			}
		}
	}

	async exists(filePath: string): Promise<boolean> {
		const fullPath = this.resolveWithinRoot(filePath);
		try {
			await fs.access(fullPath);
			return true;
		} catch {
			return false;
		}
	}
}
