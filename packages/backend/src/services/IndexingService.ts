import { Attachment, EmailDocument, PendingEmail } from '@open-archiver/types';
import { SearchService } from './SearchService';
import { StorageService } from './StorageService';
import { extractText } from '../helpers/textExtractor';
import { DatabaseService } from './DatabaseService';
import { archivedEmails, attachments, emailAttachments } from '../database/schema';
import { eq } from 'drizzle-orm';
import { streamToBuffer } from '../helpers/streamToBuffer';
import { simpleParser } from 'mailparser';
import { logger } from '../config/logger';

interface DbRecipients {
	to: { name: string; address: string }[];
	cc: { name: string; address: string }[];
	bcc: { name: string; address: string }[];
}

/**
 * Sanitizes text content by removing invalid characters that could cause JSON serialization issues
 */
function sanitizeText(text: string): string {
	if (!text) return '';

	// Remove control characters and invalid UTF-8 sequences
	return text
		.replace(/\uFFFD/g, '') // Replacement character for invalid UTF-8 sequences
		.replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, '') // Remove control characters
		.trim();
}

/**
 * Recursively sanitize all string values in an object to prevent JSON issues
 */
function sanitizeObject<T>(obj: T): T {
	if (typeof obj === 'string') {
		return sanitizeText(obj) as unknown as T;
	} else if (Array.isArray(obj)) {
		return obj.map(sanitizeObject) as unknown as T;
	} else if (obj !== null && typeof obj === 'object') {
		const sanitized: any = {};
		for (const key in obj) {
			if (Object.prototype.hasOwnProperty.call(obj, key)) {
				sanitized[key] = sanitizeObject((obj as any)[key]);
			}
		}
		return sanitized;
	}
	return obj;
}

export class IndexingService {
	private dbService: DatabaseService;
	private searchService: SearchService;
	private storageService: StorageService;

	constructor(
		dbService: DatabaseService,
		searchService: SearchService,
		storageService: StorageService
	) {
		this.dbService = dbService;
		this.searchService = searchService;
		this.storageService = storageService;
	}

	/**
	 * Index multiple emails in a single batch operation for better performance
	 */
	public async indexEmailBatch(emails: PendingEmail[]): Promise<void> {
		if (emails.length === 0) {
			return;
		}

		logger.info({ batchSize: emails.length }, 'Starting batch indexing of emails');

		try {
			const CONCURRENCY_LIMIT = 10;
			const rawDocuments: EmailDocument[] = [];

			for (let i = 0; i < emails.length; i += CONCURRENCY_LIMIT) {
				const batch = emails.slice(i, i + CONCURRENCY_LIMIT);

				const batchDocuments = await Promise.allSettled(
					batch.map(async (pendingEmail) => {
						try {
							const document = await this.indexEmailById(
								pendingEmail.archivedEmailId
							);
							if (document) {
								return document;
							}
							return null;
						} catch (error) {
							logger.error(
								{
									emailId: pendingEmail.archivedEmailId,
									error: error instanceof Error ? error.message : String(error),
								},
								'Failed to create document for email in batch'
							);
							throw error;
						}
					})
				);

				for (const result of batchDocuments) {
					if (result.status === 'fulfilled' && result.value) {
						rawDocuments.push(result.value);
					} else if (result.status === 'rejected') {
						logger.error({ error: result.reason }, 'Failed to process email in batch');
					} else {
						logger.error(
							{ result: result },
							'Failed to process email in batch, reason unknown.'
						);
					}
				}
			}

			if (rawDocuments.length === 0) {
				logger.warn('No documents created from email batch');
				return;
			}

			// Sanitize all documents
			const sanitizedDocuments = rawDocuments.map((doc) => sanitizeObject(doc));

			// Ensure all required fields are present
			const completeDocuments = sanitizedDocuments.map((doc) =>
				this.ensureEmailDocumentFields(doc)
			);

			// Validate each document and separate valid from invalid ones
			const validDocuments: EmailDocument[] = [];
			const invalidDocuments: { doc: any; reason: string }[] = [];

			for (const doc of completeDocuments) {
				if (this.isValidEmailDocument(doc)) {
					validDocuments.push(doc);
				} else {
					invalidDocuments.push({ doc, reason: 'JSON.stringify failed' });
					logger.warn({ document: doc }, 'Skipping invalid EmailDocument');
				}
			}

			// Log detailed information for invalid documents
			if (invalidDocuments.length > 0) {
				for (const { doc } of invalidDocuments) {
					logger.error(
						{
							emailId: doc.id,
							document: JSON.stringify(doc, null, 2),
						},
						'Invalid EmailDocument details'
					);
				}
			}

			if (validDocuments.length === 0) {
				logger.warn('No valid documents to index in batch.');
				return;
			}

			logger.debug({ documentCount: validDocuments.length }, 'Sending batch to Meilisearch');

			await this.searchService.addDocuments('emails', validDocuments, 'id');
			// Note: remote-content archiving is enqueued once per batch by
			// process-mailbox.processor (the only enqueuer of index-email-batch).
			// Enqueuing again here double-processed every email and raced on the
			// (emailId, urlHash) unique constraint — so it is intentionally not
			// done here. Indexing and remote-content capture are separate concerns.

			logger.info(
				{
					batchSize: emails.length,
					successfulDocuments: validDocuments.length,
					failedDocuments: emails.length - validDocuments.length,
					invalidDocuments: invalidDocuments.length,
				},
				'Successfully indexed email batch'
			);
		} catch (error) {
			logger.error(
				{
					batchSize: emails.length,
					error: error instanceof Error ? error.message : String(error),
				},
				'Failed to index email batch'
			);
			throw error;
		}
	}

	private async indexEmailById(emailId: string): Promise<EmailDocument | null> {
		const email = await this.dbService.db.query.archivedEmails.findFirst({
			where: eq(archivedEmails.id, emailId),
		});

		if (!email) {
			throw new Error(`Email with ID ${emailId} not found for indexing.`);
		}

		let emailAttachmentsResult: Attachment[] = [];
		if (email.hasAttachments) {
			emailAttachmentsResult = await this.dbService.db
				.select({
					id: attachments.id,
					filename: attachments.filename,
					mimeType: attachments.mimeType,
					sizeBytes: attachments.sizeBytes,
					contentHashSha256: attachments.contentHashSha256,
					storagePath: attachments.storagePath,
				})
				.from(emailAttachments)
				.innerJoin(attachments, eq(emailAttachments.attachmentId, attachments.id))
				.where(eq(emailAttachments.emailId, emailId));
		}

		const document = await this.createEmailDocument(
			email,
			emailAttachmentsResult,
			email.userEmail
		);
		return document;
	}

	private async createEmailDocument(
		email: typeof archivedEmails.$inferSelect,
		attachments: Attachment[],
		userEmail: string //the owner of the email inbox
	): Promise<EmailDocument> {
		const emailBodyStream = await this.storageService.get(email.storagePath);
		const emailBodyBuffer = await streamToBuffer(emailBodyStream);
		const parsedEmail = await simpleParser(emailBodyBuffer);
		const emailBodyText =
			parsedEmail.text ||
			parsedEmail.html ||
			(await extractText(emailBodyBuffer, 'text/plain')) ||
			'';

		// Attachments are stored separately with linked records; extract their text from storage.
		const attachmentContents =
			attachments.length > 0 ? await this.extractAttachmentContents(attachments) : [];

		const recipients = email.recipients as DbRecipients;
		// console.log('email.userEmail', email.userEmail);
		return {
			id: email.id,
			userEmail: userEmail,
			from: email.senderEmail,
			senderName: email.senderName || '',
			to: recipients.to?.map((r) => r.address) || [],
			cc: recipients.cc?.map((r) => r.address) || [],
			bcc: recipients.bcc?.map((r) => r.address) || [],
			subject: email.subject || '',
			body: emailBodyText,
			attachments: attachmentContents,
			timestamp: new Date(email.sentAt).getTime(),
			archivedAt: new Date(email.archivedAt).getTime(),
			ingestionSourceId: email.ingestionSourceId,
			threadId: email.threadId,
			messageIdHeader: email.messageIdHeader,
			hasAttachments: email.hasAttachments,
			sourcePath: email.sourcePath || email.path,
			sourceLabels: (email.sourceLabels as string[] | null) || [],
			tags: (email.tags as string[] | null) || [],
			sizeBytes: email.sizeBytes,
		};
	}

	private async extractAttachmentContents(
		attachments: Attachment[]
	): Promise<{ filename: string; content: string }[]> {
		const extractedAttachments = [];
		for (const attachment of attachments) {
			try {
				const fileStream = await this.storageService.get(attachment.storagePath);
				const fileBuffer = await streamToBuffer(fileStream);
				const textContent = await extractText(fileBuffer, attachment.mimeType || '');
				extractedAttachments.push({
					filename: attachment.filename,
					content: textContent,
				});
			} catch (error) {
				console.error(
					`Failed to extract text from attachment: ${attachment.filename}`,
					error
				);
			}
		}
		return extractedAttachments;
	}

	private shouldExtractText(mimeType: string): boolean {
		if (process.env.TIKA_URL) {
			return true;
		}

		if (!mimeType) return false;
		// Tika supported mime types: https://tika.apache.org/2.4.1/formats.html
		const extractableTypes = [
			'application/pdf',
			'application/msword',
			'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
			'application/vnd.ms-excel',
			'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
			'application/vnd.ms-powerpoint',
			'application/vnd.openxmlformats-officedocument.presentationml.presentation',
			'text/plain',
			'text/html',
			'application/rss+xml',
			'application/xml',
			'application/json',
			'text/rtf',
			'application/rtf',
			'text/csv',
			'text/tsv',
			'application/csv',
			'image/bpg',
			'image/png',
			'image/vnd.wap.wbmp',
			'image/x-jbig2',
			'image/bmp',
			'image/x-xcf',
			'image/gif',
			'image/x-icon',
			'image/jpeg',
			'image/x-ms-bmp',
			'image/webp',
			'image/tiff',
			'image/svg+xml',
			'application/vnd.apple.pages',
			'application/vnd.apple.numbers',
			'application/vnd.apple.keynote',
			'image/heic',
			'image/heif',
		];

		return extractableTypes.some((type) => mimeType.toLowerCase().includes(type));
	}

	/**
	 * Ensures all required fields are present in EmailDocument
	 */
	private ensureEmailDocumentFields(doc: Partial<EmailDocument>): EmailDocument {
		return {
			id: doc.id || 'missing-id',
			userEmail: doc.userEmail || 'unknown',
			from: doc.from || '',
			senderName: doc.senderName || '',
			to: Array.isArray(doc.to) ? doc.to : [],
			cc: Array.isArray(doc.cc) ? doc.cc : [],
			bcc: Array.isArray(doc.bcc) ? doc.bcc : [],
			subject: doc.subject || '',
			body: doc.body || '',
			attachments: Array.isArray(doc.attachments) ? doc.attachments : [],
			timestamp: typeof doc.timestamp === 'number' ? doc.timestamp : Date.now(),
			archivedAt: typeof doc.archivedAt === 'number' ? doc.archivedAt : Date.now(),
			ingestionSourceId: doc.ingestionSourceId || 'unknown',
			threadId: doc.threadId || null,
			messageIdHeader: doc.messageIdHeader || null,
			hasAttachments: doc.hasAttachments || false,
			sourcePath: doc.sourcePath || null,
			sourceLabels: Array.isArray(doc.sourceLabels) ? doc.sourceLabels : [],
			tags: Array.isArray(doc.tags) ? doc.tags : [],
			sizeBytes: typeof doc.sizeBytes === 'number' ? doc.sizeBytes : 0,
		};
	}

	/**
	 * Validates if the given object is a valid EmailDocument that can be serialized to JSON
	 */
	private isValidEmailDocument(doc: any): boolean {
		try {
			JSON.stringify(doc);
			return true;
		} catch (error) {
			logger.error(
				{ doc, error: (error as Error).message },
				'Invalid EmailDocument detected'
			);
			return false;
		}
	}
}
