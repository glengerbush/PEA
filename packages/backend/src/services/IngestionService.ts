import { db } from '../database';
import { ingestionSources } from '../database/schema';
import type {
	CreateIngestionSourceDto,
	UpdateIngestionSourceDto,
	IngestionSource,
	IngestionCredentials,
	IngestionProvider,
	PendingEmail,
} from '@open-archiver/types';
import { and, desc, eq, inArray, or } from 'drizzle-orm';
import { CryptoService } from './CryptoService';
import { EmailProviderFactory } from './EmailProviderFactory';
import { ingestionQueue, masterJobOptions } from '../jobs/queues';
import type { JobType } from 'bullmq';
import { StorageService } from './StorageService';
import type { EmailObject } from '@open-archiver/types';
import { stripAttachmentsFromEml } from '../helpers/emlUtils';
import { sanitizeFilename } from '../helpers/sanitize';
import {
	archivedEmails,
	attachments as attachmentsSchema,
	emailAttachments,
} from '../database/schema';
import { createHash, randomUUID } from 'crypto';
import { readFile, unlink } from 'fs/promises';
import { logger } from '../config/logger';
import { SearchService } from './SearchService';
import { config } from '../config/index';
import { User } from '@open-archiver/types';

function normalizeSourcePath(path: string | undefined): string {
	if (!path) return '';
	return path
		.split(/[\\/]+/)
		.map((part) => part.trim())
		.filter(Boolean)
		.filter((part) => part !== '.' && part !== '..')
		.join('/');
}

/**
 * Best-effort human name for a source when the user didn't provide one. For file
 * uploads it prefers the mailbox folder (`Foo.mbox`) from an uploaded path, then
 * the top-level upload folder, then a single filename; otherwise a generic
 * provider label. Prevents blank-named sources from rendering as empty rows in
 * the source dropdown.
 */
function deriveSourceName(dto: CreateIngestionSourceDto): string {
	const explicit = (dto.name ?? '').trim();
	if (explicit) return explicit;

	const config = dto.providerConfig as Record<string, any> | undefined;
	const files: Array<{ fileName?: string; relativePath?: string }> = Array.isArray(
		config?.uploadedFiles
	)
		? config!.uploadedFiles
		: [];

	for (const file of files) {
		const match = (file.relativePath || '').match(/([^/]+)\.mbox(?:\/|$)/i);
		if (match) return match[1];
	}
	const topFolder = files
		.map((file) => (file.relativePath || '').split('/')[0])
		.find((part) => part && !part.includes('.'));
	if (topFolder) return topFolder;

	const single = (config?.uploadedFileName as string) || files[0]?.fileName;
	if (single) return single.replace(/\.(mbox|eml|emlx)$/i, '');

	return `${(dto.provider || 'import').replace(/_import$/, '')} import`;
}

function normalizeDuplicateText(value: string | null | undefined): string {
	if (!value) return '';
	return value
		.replace(/<[^>]*>/g, ' ')
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, ' ')
		.trim()
		.replace(/\s+/g, ' ');
}

function duplicateHash(value: string): string | null {
	return value ? createHash('sha256').update(value).digest('hex') : null;
}

function buildFuzzyDuplicateGroupKey(
	senderEmail: string | undefined,
	subjectHash: string | null
): string | null {
	if (!senderEmail || !subjectHash) return null;
	return duplicateHash(`${senderEmail.trim().toLowerCase()}|${subjectHash}`);
}

function buildRecipientFingerprint(email: EmailObject): string | null {
	const addresses = [...(email.to || []), ...(email.cc || []), ...(email.bcc || [])]
		.map((recipient) => recipient.address?.trim().toLowerCase())
		.filter(Boolean)
		.sort();

	return addresses.length > 0 ? duplicateHash(addresses.join('|')) : null;
}

export class IngestionService {

	private static decryptSource(
		source: typeof ingestionSources.$inferSelect
	): IngestionSource | null {
		const decryptedCredentials = CryptoService.decryptObject<IngestionCredentials>(
			source.credentials as string
		);

		if (!decryptedCredentials) {
			logger.error(
				{ sourceId: source.id },
				'Failed to decrypt ingestion source credentials.'
			);
			return null;
		}

		return { ...source, credentials: decryptedCredentials } as IngestionSource;
	}

	public static returnFileBasedIngestions(): IngestionProvider[] {
		return ['eml_import', 'mbox_import'];
	}

	public static async create(
		dto: CreateIngestionSourceDto,
		userId: string,
		actor: User,
		actorIp: string
	): Promise<IngestionSource> {
		const { providerConfig, mergedIntoId, ...rest } = dto;
		const encryptedCredentials = CryptoService.encryptObject(providerConfig);

		// Resolve merge target: if mergedIntoId points to a child, follow to the root.
		let resolvedMergedIntoId: string | undefined;
		if (mergedIntoId) {
			const target = await this.findById(mergedIntoId);
			resolvedMergedIntoId = target.mergedIntoId ?? target.id;
		}

		const valuesToInsert = {
			userId,
			...rest,
			// Override any blank/missing name with a derived one (see deriveSourceName).
			name: deriveSourceName(dto),
			status: 'pending_auth' as const,
			credentials: encryptedCredentials,
			mergedIntoId: resolvedMergedIntoId ?? null,
		};

		const [newSource] = await db.insert(ingestionSources).values(valuesToInsert).returning();


		const decryptedSource = this.decryptSource(newSource);
		if (!decryptedSource) {
			await this.delete(newSource.id, actor, actorIp, true);
			throw new Error(
				'Failed to process newly created ingestion source due to a decryption error.'
			);
		}
		const connector = EmailProviderFactory.createConnector(decryptedSource);

		try {
			const connectionValid = await connector.testConnection();
			// If connection succeeds, update status to auth_success, which triggers the initial import.
			if (connectionValid) {
				return await this.update(
					decryptedSource.id,
					{ status: 'auth_success' },
					actor,
					actorIp
				);
			} else {
				throw Error('Ingestion authentication failed.');
			}
		} catch (error) {
			// If connection fails, delete the newly created source and throw the error.
			await this.delete(decryptedSource.id, actor, actorIp, true);
			throw error;
		}
	}

	public static async findAll(_userId: string): Promise<IngestionSource[]> {
		const sources = await db
			.select()
			.from(ingestionSources)
			.orderBy(desc(ingestionSources.createdAt));
		return sources.flatMap((source) => {
			const decrypted = this.decryptSource(source);
			return decrypted ? [decrypted] : [];
		});
	}

	public static async findById(id: string): Promise<IngestionSource> {
		const [source] = await db
			.select()
			.from(ingestionSources)
			.where(eq(ingestionSources.id, id));
		if (!source) {
			throw new Error('Ingestion source not found');
		}
		const decryptedSource = this.decryptSource(source);
		if (!decryptedSource) {
			throw new Error('Failed to decrypt ingestion source credentials.');
		}
		return decryptedSource;
	}

	public static async update(
		id: string,
		dto: UpdateIngestionSourceDto,
		actor?: User,
		actorIp?: string
	): Promise<IngestionSource> {
		const { providerConfig, ...rest } = dto;
		const valuesToUpdate: Partial<typeof ingestionSources.$inferInsert> = { ...rest };

		// Get the original source to compare the status later
		const originalSource = await this.findById(id);

		if (providerConfig) {
			// Encrypt the new credentials before updating
			valuesToUpdate.credentials = CryptoService.encryptObject(providerConfig);
		}

		const [updatedSource] = await db
			.update(ingestionSources)
			.set(valuesToUpdate)
			.where(eq(ingestionSources.id, id))
			.returning();

		if (!updatedSource) {
			throw new Error('Ingestion source not found');
		}

		const decryptedSource = this.decryptSource(updatedSource);

		if (!decryptedSource) {
			throw new Error(
				'Failed to process updated ingestion source due to a decryption error.'
			);
		}

		// If the status has changed to auth_success, trigger the initial import
		if (originalSource.status !== 'auth_success' && decryptedSource.status === 'auth_success') {
			await this.triggerInitialImport(decryptedSource.id);
		}
		if (actor && actorIp) {
			const changedFields = Object.keys(dto).filter(
				(key) =>
					key !== 'providerConfig' &&
					originalSource[key as keyof IngestionSource] !==
						decryptedSource[key as keyof IngestionSource]
			);
			if (changedFields.length > 0) {
			}
		}

		return decryptedSource;
	}

	/**
	 * Returns all ingestionSourceId values in a merge group given any member's ID.
	 * If the source is standalone (no parent, no children), returns just its own ID.
	 */
	public static async findGroupSourceIds(sourceId: string): Promise<string[]> {
		const [source] = await db
			.select({
				id: ingestionSources.id,
				mergedIntoId: ingestionSources.mergedIntoId,
			})
			.from(ingestionSources)
			.where(eq(ingestionSources.id, sourceId))
			.limit(1);
		if (!source) {
			throw new Error('Ingestion source not found');
		}

		const rootId = source.mergedIntoId ?? source.id;

		const children = await db
			.select({ id: ingestionSources.id })
			.from(ingestionSources)
			.where(eq(ingestionSources.mergedIntoId, rootId));

		return [rootId, ...children.map((c) => c.id)];
	}

	/**
	 * Detaches a child source from its merge group, making it standalone.
	 */
	public static async unmerge(
		id: string,
		actor: User,
		actorIp: string
	): Promise<IngestionSource> {
		const source = await this.findById(id);
		if (!source.mergedIntoId) {
			throw new Error('Source is not merged into another source.');
		}

		const [updated] = await db
			.update(ingestionSources)
			.set({ mergedIntoId: null })
			.where(eq(ingestionSources.id, id))
			.returning();


		const decrypted = this.decryptSource(updated);
		if (!decrypted) {
			throw new Error('Failed to decrypt unmerged source.');
		}
		return decrypted;
	}

	public static async delete(
		id: string,
		actor: User,
		actorIp: string,
		force: boolean = false
	): Promise<IngestionSource> {
		const source = await this.findById(id);
		if (!source) {
			throw new Error('Ingestion source not found');
		}

		// If this is a root source with children, delete all children first
		if (!source.mergedIntoId) {
			const children = await db
				.select({ id: ingestionSources.id })
				.from(ingestionSources)
				.where(eq(ingestionSources.mergedIntoId, id));

			for (const child of children) {
				await this.delete(child.id, actor, actorIp, force);
			}
		}

		// Delete all emails and attachments from storage
		const storage = new StorageService();
		const emailPath = `${config.storage.openArchiverFolderName}/${source.name.replaceAll(' ', '-')}-${source.id}/`;
		await storage.delete(emailPath);

		if (
			source.credentials.type === 'eml_import' ||
			source.credentials.type === 'mbox_import'
		) {
			const uploadedPaths = [
				...(source.credentials.uploadedFilePath
					? [source.credentials.uploadedFilePath]
					: []),
				...(source.credentials.type === 'mbox_import'
					? (source.credentials.uploadedFiles?.map((file) => file.filePath) ?? [])
					: []),
			];

			for (const uploadedPath of uploadedPaths) {
				if (await storage.exists(uploadedPath)) {
					await storage.delete(uploadedPath);
				}
			}
		}

		// Delete all emails from the database
		// NOTE: This is done by database CASADE, change when CASADE relation no longer exists.
		// await db.delete(archivedEmails).where(eq(archivedEmails.ingestionSourceId, id));

		// Delete all documents from Meilisearch
		const searchService = new SearchService();
		await searchService.deleteDocumentsByFilter('emails', `ingestionSourceId = ${id}`);

		const [deletedSource] = await db
			.delete(ingestionSources)
			.where(eq(ingestionSources.id, id))
			.returning();


		const decryptedSource = this.decryptSource(deletedSource);
		if (!decryptedSource) {
			// Even if decryption fails, we should confirm deletion.
			// We might return a simpler object or just a success message.
			// For now, we'll indicate the issue but still confirm deletion happened.
			logger.warn(
				{ sourceId: deletedSource.id },
				'Could not decrypt credentials of deleted source, but deletion was successful.'
			);
			return { ...deletedSource, credentials: null } as unknown as IngestionSource;
		}
		return decryptedSource;
	}

	public static async triggerInitialImport(id: string): Promise<void> {
		const source = await this.findById(id);

		await ingestionQueue.add('initial-import', { ingestionSourceId: source.id }, masterJobOptions);
	}

	public static async triggerForceSync(id: string, actor: User, actorIp: string): Promise<void> {
		const source = await this.findById(id);
		logger.info({ ingestionSourceId: id }, 'Force syncing started.');
		if (!source) {
			throw new Error('Ingestion source not found');
		}

		// Clean up existing jobs for this source to break any stuck flows
		const jobTypes: JobType[] = ['active', 'waiting', 'failed', 'delayed', 'paused'];
		const jobs = await ingestionQueue.getJobs(jobTypes);
		for (const job of jobs) {
			if (job.data.ingestionSourceId === id) {
				try {
					await job.remove();
					logger.info(
						{ jobId: job.id, ingestionSourceId: id },
						'Removed stale job during force sync.'
					);
				} catch (error) {
					logger.error({ err: error, jobId: job.id }, 'Failed to remove stale job.');
				}
			}
		}

		// Reset status to 'active'
		await this.update(
			id,
			{
				status: 'active',
				lastSyncStatusMessage: 'Force sync triggered by user.',
			},
			actor,
			actorIp
		);


		await ingestionQueue.add('continuous-sync', { ingestionSourceId: source.id }, masterJobOptions);

		// If this is a root source, also trigger sync for all non-file-based active/error children
		if (!source.mergedIntoId) {
			const fileBasedProviders = this.returnFileBasedIngestions();
			const children = await db
				.select({
					id: ingestionSources.id,
					provider: ingestionSources.provider,
					status: ingestionSources.status,
				})
				.from(ingestionSources)
				.where(eq(ingestionSources.mergedIntoId, id));

			for (const child of children) {
				if (
					!fileBasedProviders.includes(child.provider) &&
					(child.status === 'active' || child.status === 'error')
				) {
					logger.info(
						{ childId: child.id, parentId: id },
						'Cascading force sync to child source.'
					);
					await ingestionQueue.add('continuous-sync', { ingestionSourceId: child.id }, masterJobOptions);
				}
			}
		}
	}

	/**
	 * Pre-fetch duplicate check to avoid unnecessary API calls during ingestion.
	 * Checks both providerMessageId (for Google/Microsoft API IDs) and
	 * messageIdHeader (for IMAP/PST/EML/Mbox RFC Message-IDs).
	 *
	 * The check is scoped to the full merge group so that emails already archived
	 * by a sibling source are not re-downloaded and stored again.
	 */
	public static async doesEmailExist(
		messageId: string,
		ingestionSourceId: string
	): Promise<boolean> {
		const groupIds = await this.findGroupSourceIds(ingestionSourceId);
		const sourceFilter =
			groupIds.length === 1
				? eq(archivedEmails.ingestionSourceId, groupIds[0])
				: inArray(archivedEmails.ingestionSourceId, groupIds);

		const existingEmail = await db.query.archivedEmails.findFirst({
			where: and(
				sourceFilter,
				or(
					eq(archivedEmails.providerMessageId, messageId),
					eq(archivedEmails.messageIdHeader, messageId)
				)
			),
			columns: { id: true },
		});
		return !!existingEmail;
	}

	public async processEmail(
		email: EmailObject,
		source: IngestionSource,
		storage: StorageService,
		userEmail: string
	): Promise<PendingEmail | null> {
		try {
			// Read the raw bytes from the temp file written by the connector
			const rawEmlBuffer = await readFile(email.tempFilePath);

			// If this source is a child in a merge group, redirect all storage and DB
			// ownership to the root source. Child sources are "assistants" — they fetch
			// emails on behalf of the root but never own any stored content.
			const effectiveSource = source.mergedIntoId
				? await IngestionService.findById(source.mergedIntoId)
				: source;

			// Generate a unique message ID for the email. If the email already has a message-id header, use that.
			// Otherwise, generate a new one based on the email's hash, source ID, and email ID.
			const messageIdHeader = email.headers.get('message-id');
			let messageId: string | undefined;
			if (Array.isArray(messageIdHeader)) {
				messageId = messageIdHeader[0];
			} else if (typeof messageIdHeader === 'string') {
				messageId = messageIdHeader;
			}
			if (!messageId) {
				messageId = `generated-${createHash('sha256')
					.update(rawEmlBuffer)
					.digest('hex')}-${source.id}-${email.id}`;
			}
			// Check if an email with the same message ID has already been imported
			// within the merge group. This prevents duplicate imports when the same
			// email exists in multiple mailboxes or across merged ingestion sources.
			const groupIds = await IngestionService.findGroupSourceIds(source.id);
			const groupSourceFilter =
				groupIds.length === 1
					? eq(archivedEmails.ingestionSourceId, groupIds[0])
					: inArray(archivedEmails.ingestionSourceId, groupIds);

			const existingEmail = await db.query.archivedEmails.findFirst({
				where: and(eq(archivedEmails.messageIdHeader, messageId), groupSourceFilter),
			});

			if (existingEmail) {
				logger.info(
					{ messageId, ingestionSourceId: source.id },
					'Skipping duplicate email'
				);
				return null;
			}

			const sourcePath = normalizeSourcePath(email.path);
			const sourceLabels = email.tags || [];
			// Fold the original folder (sourcePath) into tags — folders are just tags now.
			const emailTags = sourcePath
				? Array.from(new Set([...sourceLabels, sourcePath]))
				: sourceLabels;
			const senderEmail = email.from[0]?.address || '';
			const duplicateSubjectHash = duplicateHash(normalizeDuplicateText(email.subject));
			const duplicateFuzzyGroupKey = buildFuzzyDuplicateGroupKey(
				senderEmail,
				duplicateSubjectHash
			);
			const duplicateBodyHash = duplicateHash(
				normalizeDuplicateText(email.body || email.html)
			);
			const duplicateRecipientFingerprint = buildRecipientFingerprint(email);
			let duplicateAttachmentFingerprint: string | null = null;
			const storagePathSegment = sourcePath ? `${sourcePath}/` : '';
			// Use effectiveSource (root) for storage path and DB ownership.
			// Child sources are assistants; all content physically belongs to the root.
			const emailPath = `${config.storage.openArchiverFolderName}/${effectiveSource.name.replaceAll(' ', '-')}-${effectiveSource.id}/emails/${storagePathSegment}${email.id}.eml`;

			// Strip non-inline attachments from the .eml to avoid double-storing
			// attachment data (attachments are stored separately).
			const emlBuffer = await stripAttachmentsFromEml(rawEmlBuffer);
			const emailHash = createHash('sha256').update(emlBuffer).digest('hex');
			await storage.put(emailPath, emlBuffer);

			const [archivedEmail] = await db
				.insert(archivedEmails)
				.values({
					// Always assign to root (effectiveSource)
					ingestionSourceId: effectiveSource.id,
					userEmail,
					threadId: email.threadId,
					messageIdHeader: messageId,
					providerMessageId: email.id,
					sentAt: email.receivedAt,
					subject: email.subject,
					senderName: email.from[0]?.name,
					senderEmail,
					recipients: {
						to: email.to,
						cc: email.cc,
						bcc: email.bcc,
					},
					storagePath: emailPath,
					storageHashSha256: emailHash,
					sizeBytes: emlBuffer.length,
					hasAttachments: email.attachments.length > 0,
					sourcePath,
					sourceLabels,
					duplicateSubjectHash,
					duplicateFuzzyGroupKey,
					duplicateBodyHash,
					duplicateRecipientFingerprint,
					duplicateAttachmentFingerprint,
					path: sourcePath,
					tags: emailTags,
				})
				.returning();

			if (email.attachments.length > 0) {
				const attachmentHashes: string[] = [];
				for (const attachment of email.attachments) {
					const attachmentBuffer = attachment.content;
					const attachmentHash = createHash('sha256')
						.update(attachmentBuffer)
						.digest('hex');
					attachmentHashes.push(attachmentHash);

					// Check if an attachment with the same hash already exists for the root source
					const existingAttachment = await db.query.attachments.findFirst({
						where: and(
							eq(attachmentsSchema.contentHashSha256, attachmentHash),
							eq(attachmentsSchema.ingestionSourceId, effectiveSource.id)
						),
					});

					let attachmentId: string;

					if (existingAttachment) {
						attachmentId = existingAttachment.id;
						logger.info(
							{
								attachmentHash,
								ingestionSourceId: effectiveSource.id,
								reusedPath: existingAttachment.storagePath,
							},
							'Reusing existing attachment file for deduplication.'
						);
					} else {
						// New attachment: store under the root source's folder
						const uniqueId = randomUUID().slice(0, 7);
						const storagePath = `${config.storage.openArchiverFolderName}/${effectiveSource.name.replaceAll(' ', '-')}-${effectiveSource.id}/attachments/${uniqueId}-${sanitizeFilename(attachment.filename)}`;
						await storage.put(storagePath, attachmentBuffer);

						const [newRecord] = await db
							.insert(attachmentsSchema)
							.values({
								filename: attachment.filename,
								mimeType: attachment.contentType,
								sizeBytes: attachment.size,
								contentHashSha256: attachmentHash,
								storagePath,
								// Always assign attachment ownership to root (effectiveSource)
								ingestionSourceId: effectiveSource.id,
							})
							.returning();
						attachmentId = newRecord.id;
					}

					// Link the attachment record (either new or existing) to the email.
					// One row per occurrence: two byte-identical attachments in the same
					// email dedupe to one attachment record but must keep two links, so
					// both survive reconstruction (the link table has a surrogate PK).
					await db.insert(emailAttachments).values({
						emailId: archivedEmail.id,
						attachmentId,
					});
				}

				duplicateAttachmentFingerprint = duplicateHash(attachmentHashes.sort().join('|'));
				await db
					.update(archivedEmails)
					.set({ duplicateAttachmentFingerprint })
					.where(eq(archivedEmails.id, archivedEmail.id));
			}

			return {
				archivedEmailId: archivedEmail.id,
			};
		} catch (error) {
			logger.error({
				message: `Failed to process email ${email.id} for source ${source.id}`,
				error,
				emailId: email.id,
				ingestionSourceId: source.id,
			});
			return null;
		} finally {
			// Always clean up the temp file, regardless of success or failure
			await unlink(email.tempFilePath).catch((err) =>
				logger.warn(
					{ err, tempFilePath: email.tempFilePath },
					'Failed to delete temp email file'
				)
			);
		}
	}
}
