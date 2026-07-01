import type {
	MboxImportCredentials,
	EmailObject,
	EmailAddress,
	SyncState,
	MailboxUser,
} from '@open-archiver/types';
import type { IEmailConnector } from '../EmailProviderFactory';
import { simpleParser, ParsedMail, Attachment, AddressObject } from 'mailparser';
import { logger } from '../../config/logger';
import { getThreadId } from './helpers/utils';
import { writeEmailToTempFile } from './helpers/tempFile';
import { StorageService } from '../StorageService';
import { Transform } from 'stream';
import { createHash } from 'crypto';
import { promises as fs, createReadStream } from 'fs';
import { basename, join, relative } from 'path';
import { streamToBuffer } from '../../helpers/streamToBuffer';

type MboxInput = {
	filePath: string;
	sourcePath: string;
	isLocal: boolean;
	format: 'mbox' | 'emlx';
};

class MboxSplitter extends Transform {
	private buffer: Buffer = Buffer.alloc(0);
	private delimiter: Buffer = Buffer.from('\nFrom ');

	_transform(chunk: Buffer, encoding: string, callback: Function) {
		let currentBuffer = Buffer.concat([this.buffer, chunk]);
		let position;

		while ((position = currentBuffer.indexOf(this.delimiter)) > -1) {
			const email = currentBuffer.subarray(0, position);
			if (email.length > 0) {
				this.push(email);
			}
			// The next email starts with "From ", which is what the parser expects.
			currentBuffer = currentBuffer.subarray(position + 1);
		}

		this.buffer = currentBuffer;
		callback();
	}

	_flush(callback: Function) {
		if (this.buffer.length > 0) {
			this.push(this.buffer);
		}
		callback();
	}
}

export class MboxConnector implements IEmailConnector {
	private storage: StorageService;

	constructor(private credentials: MboxImportCredentials) {
		this.storage = new StorageService();
	}

	public async testConnection(): Promise<boolean> {
		try {
			await this.getMboxInputs();
			return true;
		} catch (error) {
			logger.error({ error, credentials: this.credentials }, 'Mbox file validation failed.');
			throw error;
		}
	}

	private getFilePath(): string {
		return this.credentials.localFilePath || this.credentials.uploadedFilePath || '';
	}

	private isMboxPath(filePath: string): boolean {
		return filePath.toLowerCase().endsWith('.mbox');
	}

	private isEmlxPath(filePath: string): boolean {
		return filePath.toLowerCase().endsWith('.emlx');
	}

	private stripMboxExtension(filePath: string): string {
		return filePath.replace(/\.mbox$/i, '');
	}

	private toSourcePath(filePath: string): string {
		return this.stripMboxExtension(filePath)
			.split(/[\\/]+/)
			.filter(Boolean)
			.join('/');
	}

	private toAppleMailSourcePath(filePath: string): string {
		return filePath
			.split(/[\\/]+/)
			.filter((segment) => this.isMboxPath(segment))
			.map((segment) => this.stripMboxExtension(segment))
			.join('/');
	}

	private getAppleMailSourcePath(importRoot: string, filePath: string): string {
		return this.toAppleMailSourcePath(relative(importRoot, filePath));
	}

	private async findLocalMboxInputs(
		importRoot: string,
		directoryPath: string = importRoot
	): Promise<MboxInput[]> {
		const entries = await fs.readdir(directoryPath, { withFileTypes: true });
		const inputs: MboxInput[] = [];

		for (const entry of entries) {
			const entryPath = join(directoryPath, entry.name);

			if (entry.isDirectory()) {
				inputs.push(...(await this.findLocalMboxInputs(importRoot, entryPath)));
			} else if (entry.isFile() && this.isMboxPath(entry.name)) {
				inputs.push({
					filePath: entryPath,
					sourcePath: this.toSourcePath(relative(importRoot, entryPath)),
					isLocal: true,
					format: 'mbox',
				});
			} else if (
				entry.isFile() &&
				this.isEmlxPath(entry.name) &&
				(this.isMboxPath(importRoot) ||
					relative(importRoot, entryPath)
						.split(/[\\/]+/)
						.some((segment) => this.isMboxPath(segment)))
			) {
				inputs.push({
					filePath: entryPath,
					sourcePath: this.getAppleMailSourcePath(importRoot, entryPath),
					isLocal: true,
					format: 'emlx',
				});
			}
		}

		return inputs.sort((a, b) => a.filePath.localeCompare(b.filePath));
	}

	private async getMboxInputs(): Promise<MboxInput[]> {
		const filePath = this.getFilePath();
		if (!filePath && !this.credentials.uploadedFiles?.length) {
			throw Error('Mbox file or folder path not provided.');
		}

		if (this.credentials.localFilePath) {
			let stats;
			try {
				stats = await fs.stat(this.credentials.localFilePath);
			} catch {
				throw Error(
					`Mbox file or folder not found inside the OpenArchiver server at path: ${this.credentials.localFilePath}`
				);
			}

			if (stats.isDirectory()) {
				const inputs = await this.findLocalMboxInputs(this.credentials.localFilePath);
				if (inputs.length === 0) {
					throw Error(
						`No mbox files or Apple Mail messages found under directory: ${this.credentials.localFilePath}`
					);
				}

				return inputs;
			}

			if (!stats.isFile()) {
				throw Error(
					`Mbox path is not a file or directory: ${this.credentials.localFilePath}`
				);
			}

			if (!this.isMboxPath(this.credentials.localFilePath)) {
				throw Error('Provided local file is not in the MBOX format.');
			}

			return [
				{
					filePath: this.credentials.localFilePath,
					sourcePath: '',
					isLocal: true,
					format: 'mbox',
				},
			];
		}

		if (this.credentials.uploadedFiles?.length) {
			const uploadedFiles = this.credentials.uploadedFiles;
			for (const uploadedFile of uploadedFiles) {
				if (
					!this.isMboxPath(uploadedFile.fileName) &&
					!this.isEmlxPath(uploadedFile.fileName)
				) {
					throw Error(
						`Uploaded file is not an MBOX or Apple Mail EMLX file: ${uploadedFile.fileName}`
					);
				}
				if (!(await this.storage.exists(uploadedFile.filePath))) {
					throw Error(`Uploaded Mbox file not found: ${uploadedFile.fileName}`);
				}
			}

			const useFileNamesAsPaths = uploadedFiles.length > 1;
			return uploadedFiles.map((uploadedFile) => {
				const format = this.isEmlxPath(uploadedFile.fileName) ? 'emlx' : 'mbox';
				return {
					filePath: uploadedFile.filePath,
					sourcePath:
						format === 'emlx'
							? this.toAppleMailSourcePath(
									uploadedFile.relativePath || uploadedFile.fileName
								)
							: useFileNamesAsPaths
								? this.toSourcePath(uploadedFile.fileName)
								: '',
					isLocal: false,
					format,
				};
			});
		}

		if (!this.isMboxPath(filePath)) {
			throw Error('Provided file is not in the MBOX format.');
		}

		const fileExists = await this.storage.exists(filePath);
		if (!fileExists) {
			throw Error(
				'Uploaded Mbox file not found. The upload may not have finished yet, or it failed.'
			);
		}

		return [{ filePath, sourcePath: '', isLocal: false, format: 'mbox' }];
	}

	private async getFileStream(input: MboxInput): Promise<NodeJS.ReadableStream> {
		if (input.isLocal) {
			return createReadStream(input.filePath);
		}
		return this.storage.getStream(input.filePath);
	}

	public async *listAllUsers(): AsyncGenerator<MailboxUser> {
		const displayName = this.getDisplayName();
		logger.info(`Found potential mailbox: ${displayName}`);
		const constructedPrimaryEmail = `${displayName.replace(/ /g, '.').toLowerCase()}@mbox.local`;
		yield {
			id: constructedPrimaryEmail,
			primaryEmail: constructedPrimaryEmail,
			displayName: displayName,
		};
	}

	private getDisplayName(): string {
		if (this.credentials.uploadedFiles?.length) {
			return this.credentials.uploadedFiles.length === 1
				? this.credentials.uploadedFiles[0].fileName
				: `${this.credentials.uploadedFiles.length}-file-mbox-import`;
		}
		if (this.credentials.uploadedFileName) {
			return this.credentials.uploadedFileName;
		}
		if (this.credentials.localFilePath) {
			return this.stripMboxExtension(basename(this.credentials.localFilePath));
		}
		return `mbox-import-${new Date().getTime()}`;
	}

	public async *fetchEmails(
		userEmail: string,
		syncState?: SyncState | null
	): AsyncGenerator<EmailObject | null> {
		const inputs = await this.getMboxInputs();
		const seenAppleMailMessages = new Set<string>();

		try {
			for (const input of inputs) {
				try {
					if (input.format === 'emlx') {
						const emlxBuffer = await streamToBuffer(await this.getFileStream(input));
						const emailBuffer = this.extractEmlxMessage(emlxBuffer, input.filePath);
						const messageHash = createHash('sha256').update(emailBuffer).digest('hex');

						if (seenAppleMailMessages.has(messageHash)) {
							continue;
						}
						seenAppleMailMessages.add(messageHash);
						yield await this.parseMessage(emailBuffer, input.sourcePath);
						continue;
					}

					const fileStream = await this.getFileStream(input);
					const emailStream = fileStream.pipe(new MboxSplitter());
					for await (const emailBuffer of emailStream) {
						try {
							// mbox-only transport cleanup: strip the "From " envelope
							// line and reverse ">From " quoting before parsing/hashing.
							const emlBuffer = this.unescapeMboxQuoting(
								this.stripMboxEnvelope(emailBuffer as Buffer)
							);
							yield await this.parseMessage(emlBuffer, input.sourcePath);
						} catch (error) {
							logger.error(
								{ error, file: input.filePath },
								'Failed to process a single message from mbox file. Skipping.'
							);
						}
					}
				} catch (error) {
					logger.error(
						{ error, file: input.filePath },
						'Failed to process an mbox input. Skipping.'
					);
				}
			}
		} finally {
			if (!this.credentials.localFilePath) {
				const uploadedPaths = this.credentials.uploadedFiles?.length
					? this.credentials.uploadedFiles.map((file) => file.filePath)
					: this.credentials.uploadedFilePath
						? [this.credentials.uploadedFilePath]
						: [];

				for (const uploadedPath of uploadedPaths) {
					try {
						await this.storage.delete(uploadedPath);
					} catch (error) {
						logger.error(
							{ error, file: uploadedPath },
							'Failed to delete mbox file after processing.'
						);
					}
				}
			}
		}
	}

	private extractEmlxMessage(buffer: Buffer, filePath: string): Buffer {
		const newlineIndex = buffer.indexOf(0x0a);
		if (newlineIndex === -1) {
			throw new Error(`Invalid Apple Mail EMLX file (missing length line): ${filePath}`);
		}

		const lengthLine = buffer.subarray(0, newlineIndex).toString('ascii').trim();
		if (!/^\d+$/.test(lengthLine)) {
			throw new Error(`Invalid Apple Mail EMLX file (invalid length): ${filePath}`);
		}

		const messageLength = Number(lengthLine);
		const messageStart = newlineIndex + 1;
		const messageEnd = messageStart + messageLength;
		if (
			!Number.isSafeInteger(messageLength) ||
			messageLength <= 0 ||
			messageEnd > buffer.length
		) {
			throw new Error(`Invalid Apple Mail EMLX file (truncated message): ${filePath}`);
		}

		return buffer.subarray(messageStart, messageEnd);
	}

	/**
	 * Strips the mbox "From " envelope line from the raw buffer.
	 * The mbox format prepends each message with a "From sender@... timestamp\n"
	 * line that is NOT part of the RFC 5322 message. Storing this line in the
	 * .eml would produce an invalid file and corrupt the SHA-256 content hash.
	 */
	private stripMboxEnvelope(buffer: Buffer): Buffer {
		// The "From " line ends at the first \n — everything after is the real RFC 5322 message.
		const fromPrefix = Buffer.from('From ');
		if (buffer.subarray(0, fromPrefix.length).equals(fromPrefix)) {
			const newlineIndex = buffer.indexOf(0x0a); // \n
			if (newlineIndex !== -1) {
				return buffer.subarray(newlineIndex + 1);
			}
		}
		return buffer;
	}

	/**
	 * Reverses mbox "From " quoting: an escaped ">From " becomes "From " (and a
	 * ">>From " becomes ">From "), by stripping exactly one leading ">" from every
	 * line matching /^>+From /. This is the mboxrd rule and the modern default;
	 * it also restores the common mboxo case. It cannot distinguish a genuinely
	 * quoted ">From " in an mboxo file (an inherent, undecidable ambiguity), but
	 * without it the stored .eml and its SHA-256 hash are corrupted for the far
	 * more common escaped case. Operates on latin1 so byte content is preserved 1:1.
	 * Only applied to the mbox path — never to verbatim emlx messages.
	 */
	private unescapeMboxQuoting(buffer: Buffer): Buffer {
		const text = buffer.toString('latin1');
		const unescaped = text.replace(/^>(>*From )/gm, '$1');
		return Buffer.from(unescaped, 'latin1');
	}

	// Parses an already-clean RFC 5322 message buffer. Callers must remove any
	// transport framing first: the mbox path strips the "From " envelope and
	// reverses ">From " quoting, whereas the emlx path passes Apple Mail's verbatim
	// message (which is NOT mbox-quoted and must not be de-quoted).
	private async parseMessage(emlBuffer: Buffer, path: string): Promise<EmailObject> {
		const parsedEmail: ParsedMail = await simpleParser(emlBuffer);
		const hasContent =
			parsedEmail.headers.size > 0 ||
			Boolean(parsedEmail.text?.trim()) ||
			(typeof parsedEmail.html === 'string' && parsedEmail.html.trim().length > 0);
		if (!hasContent) {
			throw new Error('Mbox message did not contain headers or body content.');
		}

		const tempFilePath = await writeEmailToTempFile(emlBuffer);

		const attachments = parsedEmail.attachments.map((attachment: Attachment) => ({
			filename: attachment.filename || 'untitled',
			contentType: attachment.contentType,
			size: attachment.size,
			content: attachment.content as Buffer,
		}));

		const mapAddresses = (
			addresses: AddressObject | AddressObject[] | undefined
		): EmailAddress[] => {
			if (!addresses) return [];
			const addressArray = Array.isArray(addresses) ? addresses : [addresses];
			return addressArray.flatMap((a) =>
				a.value.map((v) => ({
					name: v.name,
					address: v.address?.replaceAll(`'`, '') || '',
				}))
			);
		};

		const threadId = getThreadId(parsedEmail.headers);
		let messageId = parsedEmail.messageId;

		if (!messageId) {
			messageId = `generated-${createHash('sha256').update(emlBuffer).digest('hex')}`;
		}

		const from = mapAddresses(parsedEmail.from);
		if (from.length === 0) {
			from.push({ name: 'No Sender', address: 'No Sender' });
		}

		// Extract folder path from headers. Mbox files don't have a standard folder structure, so we rely on custom headers added by email clients.
		// Gmail uses 'X-Gmail-Labels', and other clients like Thunderbird may use 'X-Folder'.
		const gmailLabels = parsedEmail.headers.get('x-gmail-labels');
		const folderHeader = parsedEmail.headers.get('x-folder');
		let finalPath = path;

		if (gmailLabels && typeof gmailLabels === 'string') {
			// We take the first label as the primary folder.
			// Gmail labels can be hierarchical, but we'll simplify to the first label.
			finalPath = gmailLabels.split(',')[0];
		} else if (folderHeader && typeof folderHeader === 'string') {
			finalPath = folderHeader;
		}

		return {
			id: messageId,
			threadId: threadId,
			from,
			to: mapAddresses(parsedEmail.to),
			cc: mapAddresses(parsedEmail.cc),
			bcc: mapAddresses(parsedEmail.bcc),
			subject: parsedEmail.subject || '',
			body: parsedEmail.text || '',
			html: parsedEmail.html || '',
			headers: parsedEmail.headers,
			attachments,
			receivedAt: parsedEmail.date || new Date(),
			tempFilePath,
			path: finalPath,
		};
	}

	public getUpdatedSyncState(): SyncState {
		return {};
	}
}
