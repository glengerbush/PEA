import { count, desc, eq, asc, and, inArray } from 'drizzle-orm';
import { db } from '../database';
import {
	archivedEmails,
	attachments,
	emailAttachments,
	ingestionSources,
} from '../database/schema';
import type {
	PaginatedArchivedEmails,
	ArchivedEmail,
	Recipient,
	ThreadEmail,
	RemoteContentStatus,
} from '@open-archiver/types';
import { StorageService } from './StorageService';
import { SearchService } from './SearchService';
import { IngestionService } from './IngestionService';
import type { Readable } from 'stream';
import { User } from '@open-archiver/types';
import { logger } from '../config/logger';

interface DbRecipients {
	to: { name: string; address: string }[];
	cc: { name: string; address: string }[];
	bcc: { name: string; address: string }[];
}

async function streamToBuffer(stream: Readable): Promise<Buffer> {
	return new Promise((resolve, reject) => {
		const chunks: Buffer[] = [];
		stream.on('data', (chunk) => chunks.push(chunk));
		stream.on('error', reject);
		stream.on('end', () => resolve(Buffer.concat(chunks)));
	});
}

export class ArchivedEmailService {
	private static mapRecipients(dbRecipients: unknown): Recipient[] {
		const { to = [], cc = [], bcc = [] } = dbRecipients as DbRecipients;

		const allRecipients = [...to, ...cc, ...bcc];

		return allRecipients.map((r) => ({
			name: r.name,
			email: r.address,
		}));
	}

	public static async getArchivedEmails(
		ingestionSourceId: string,
		page: number,
		limit: number,
		_userId: string
	): Promise<PaginatedArchivedEmails> {
		const offset = (page - 1) * limit;

		// Expand to the full merge group so emails from children appear when browsing a root source
		const groupIds = await IngestionService.findGroupSourceIds(ingestionSourceId);
		const where =
			groupIds.length === 1
				? eq(archivedEmails.ingestionSourceId, groupIds[0])
				: inArray(archivedEmails.ingestionSourceId, groupIds);

		const countQuery = db
			.select({
				count: count(archivedEmails.id),
			})
			.from(archivedEmails)
			.leftJoin(ingestionSources, eq(archivedEmails.ingestionSourceId, ingestionSources.id));

		if (where) {
			countQuery.where(where);
		}

		const [total] = await countQuery;

		const itemsQuery = db
			.select()
			.from(archivedEmails)
			.leftJoin(ingestionSources, eq(archivedEmails.ingestionSourceId, ingestionSources.id))
			.orderBy(desc(archivedEmails.sentAt))
			.limit(limit)
			.offset(offset);

		if (where) {
			itemsQuery.where(where);
		}

		const results = await itemsQuery;
		const items = results.map((r) => r.archived_emails);

		return {
			items: items.map((item) => ({
				...item,
				recipients: this.mapRecipients(item.recipients),
				sourceLabels: (item.sourceLabels as string[] | null) || null,
				remoteContentStatus: item.remoteContentStatus as RemoteContentStatus,
				tags: (item.tags as string[] | null) || null,
				path: item.path || null,
			})),
			total: total.count,
			page,
			limit,
		};
	}

	public static async getArchivedEmailById(
		emailId: string,
		_userId: string,
		actor: User,
		actorIp: string
	): Promise<ArchivedEmail | null> {
		const email = await db.query.archivedEmails.findFirst({
			where: eq(archivedEmails.id, emailId),
			with: {
				ingestionSource: true,
			},
		});

		if (!email) {
			return null;
		}


		let threadEmails: ThreadEmail[] = [];

		// Expand thread query to the full merge group so threads can span across merged sources
		if (email.threadId) {
			const groupIds = await IngestionService.findGroupSourceIds(email.ingestionSourceId);
			const sourceFilter =
				groupIds.length === 1
					? eq(archivedEmails.ingestionSourceId, groupIds[0])
					: inArray(archivedEmails.ingestionSourceId, groupIds);
			threadEmails = await db.query.archivedEmails.findMany({
				where: and(eq(archivedEmails.threadId, email.threadId), sourceFilter),
				orderBy: [asc(archivedEmails.sentAt)],
				columns: {
					id: true,
					subject: true,
					sentAt: true,
					senderEmail: true,
					hasAttachments: true,
				},
			});
		}

		const storage = new StorageService();
		const rawStream = await storage.get(email.storagePath);
		const raw = await streamToBuffer(rawStream as Readable);

		const mappedEmail = {
			...email,
			recipients: this.mapRecipients(email.recipients),
			raw,
			thread: threadEmails,
			sourceLabels: (email.sourceLabels as string[] | null) || null,
			remoteContentStatus: email.remoteContentStatus as RemoteContentStatus,
			tags: (email.tags as string[] | null) || null,
			path: email.path || null,
		};

		if (email.hasAttachments) {
			const emailAttachmentsResult = await db
				.select({
					id: attachments.id,
					filename: attachments.filename,
					mimeType: attachments.mimeType,
					sizeBytes: attachments.sizeBytes,
					storagePath: attachments.storagePath,
				})
				.from(emailAttachments)
				.innerJoin(attachments, eq(emailAttachments.attachmentId, attachments.id))
				.where(eq(emailAttachments.emailId, emailId));

			// const attachmentsWithRaw = await Promise.all(
			//     emailAttachmentsResult.map(async (attachment) => {
			//         const rawStream = await storage.get(attachment.storagePath);
			//         const raw = await streamToBuffer(rawStream as Readable);
			//         return { ...attachment, raw };
			//     })
			// );

			return {
				...mappedEmail,
				attachments: emailAttachmentsResult,
			};
		}

		return mappedEmail;
	}

	public static async deleteArchivedEmail(
		emailId: string,
		actor: User,
		actorIp: string
	): Promise<void> {
		const [email] = await db
			.select()
			.from(archivedEmails)
			.where(eq(archivedEmails.id, emailId));

		if (!email) {
			throw new Error('Archived email not found');
		}

		const storage = new StorageService();

		// Load and handle attachments before deleting the email itself
		if (email.hasAttachments) {
			const attachmentsForEmail = await db
				.select({
					attachmentId: attachments.id,
					storagePath: attachments.storagePath,
				})
				.from(emailAttachments)
				.innerJoin(attachments, eq(emailAttachments.attachmentId, attachments.id))
				.where(eq(emailAttachments.emailId, emailId));

			try {
				for (const attachment of attachmentsForEmail) {
					// Delete the link between this email and the attachment record.
					await db
						.delete(emailAttachments)
						.where(
							and(
								eq(emailAttachments.emailId, emailId),
								eq(emailAttachments.attachmentId, attachment.attachmentId)
							)
						);

					// Check if any other emails are linked to this attachment record.
					const [recordRefCount] = await db
						.select({ count: count() })
						.from(emailAttachments)
						.where(eq(emailAttachments.attachmentId, attachment.attachmentId));

					// If no other emails are linked to this record, it's safe to delete it and the file.
					if (recordRefCount.count === 0) {
						await storage.delete(attachment.storagePath);
						await db
							.delete(attachments)
							.where(eq(attachments.id, attachment.attachmentId));
					}
				}
			} catch (error) {
				logger.error(
					{
						emailId,
						error: error instanceof Error ? error.message : String(error),
					},
					'Failed to delete email attachments'
				);
				throw new Error('Failed to delete email attachments');
			}
		}

		// Delete the email file from storage
		await storage.delete(email.storagePath);

		const searchService = new SearchService();
		await searchService.deleteDocuments('emails', [emailId]);

		await db.delete(archivedEmails).where(eq(archivedEmails.id, emailId));

		// Auto-remove an emptied source, but only for a finished file-based import
		// (mbox/eml). Pending/syncing/remote sources may legitimately sit at 0 emails,
		// so they're left alone. Failure here must not fail the (already-done) email
		// deletion, so it's best-effort.
		if (email.ingestionSourceId) {
			try {
				const [remaining] = await db
					.select({ count: count() })
					.from(archivedEmails)
					.where(eq(archivedEmails.ingestionSourceId, email.ingestionSourceId));

				if (remaining.count === 0) {
					const [source] = await db
						.select({
							provider: ingestionSources.provider,
							status: ingestionSources.status,
						})
						.from(ingestionSources)
						.where(eq(ingestionSources.id, email.ingestionSourceId));

					const fileBasedProviders = IngestionService.returnFileBasedIngestions();
					const isTerminal = source?.status === 'imported' || source?.status === 'error';
					if (source && fileBasedProviders.includes(source.provider) && isTerminal) {
						await IngestionService.delete(email.ingestionSourceId, actor, actorIp, true);
						logger.info(
							{ ingestionSourceId: email.ingestionSourceId },
							'Auto-deleted emptied file-based ingestion source after its last email was removed.'
						);
					}
				}
			} catch (cleanupError) {
				logger.warn(
					{ err: cleanupError, ingestionSourceId: email.ingestionSourceId },
					'Failed to auto-clean emptied ingestion source (email deletion still succeeded).'
				);
			}
		}
	}
}
