import { relations } from 'drizzle-orm';
import {
	boolean,
	jsonb,
	pgTable,
	text,
	timestamp,
	uuid,
	bigint,
	index,
	type AnyPgColumn,
} from 'drizzle-orm/pg-core';
import { ingestionSources } from './ingestion-sources';
import { archiveFolders } from './archive-folders';

export const archivedEmails = pgTable(
	'archived_emails',
	{
		id: uuid('id').primaryKey().defaultRandom(),
		threadId: text('thread_id'),
		ingestionSourceId: uuid('ingestion_source_id')
			.notNull()
			.references(() => ingestionSources.id, { onDelete: 'cascade' }),
		userEmail: text('user_email').notNull(),
		messageIdHeader: text('message_id_header'),
		/** The provider-specific message ID (e.g., Gmail API ID, Graph API ID).
		 * Used by the pre-fetch duplicate check to avoid unnecessary API calls during retries. */
		providerMessageId: text('provider_message_id'),
		sentAt: timestamp('sent_at', { withTimezone: true }).notNull(),
		subject: text('subject'),
		senderName: text('sender_name'),
		senderEmail: text('sender_email').notNull(),
		recipients: jsonb('recipients'),
		storagePath: text('storage_path').notNull(),
		storageHashSha256: text('storage_hash_sha256').notNull(),
		sizeBytes: bigint('size_bytes', { mode: 'number' }).notNull(),
		isIndexed: boolean('is_indexed').notNull().default(false),
		hasAttachments: boolean('has_attachments').notNull().default(false),
		isOnLegalHold: boolean('is_on_legal_hold').notNull().default(false),
		isJournaled: boolean('is_journaled').default(false),
		archivedAt: timestamp('archived_at', { withTimezone: true }).notNull().defaultNow(),
		sourcePath: text('source_path'),
		sourceLabels: jsonb('source_labels'),
		localFolderId: uuid('local_folder_id').references(() => archiveFolders.id, {
			onDelete: 'set null',
		}),
		localFolderPath: text('local_folder_path'),
		duplicateOfEmailId: uuid('duplicate_of_email_id').references(
			(): AnyPgColumn => archivedEmails.id,
			{
				onDelete: 'set null',
			}
		),
		duplicateReviewStatus: text('duplicate_review_status').notNull().default('unique'),
		isDuplicateHidden: boolean('is_duplicate_hidden').notNull().default(false),
		duplicateSubjectHash: text('duplicate_subject_hash'),
		duplicateFuzzyGroupKey: text('duplicate_fuzzy_group_key'),
		duplicateBodyHash: text('duplicate_body_hash'),
		duplicateRecipientFingerprint: text('duplicate_recipient_fingerprint'),
		duplicateAttachmentFingerprint: text('duplicate_attachment_fingerprint'),
		remoteContentStatus: text('remote_content_status').notNull().default('not_started'),
		remoteContentAssetCount: bigint('remote_content_asset_count', { mode: 'number' })
			.notNull()
			.default(0),
		remoteContentArchivedAt: timestamp('remote_content_archived_at', {
			withTimezone: true,
		}),
		path: text('path'),
		tags: jsonb('tags'),
	},
	(table) => [
		index('thread_id_idx').on(table.threadId),
		index('archived_emails_message_id_header_idx').on(table.messageIdHeader),
		index('archived_emails_storage_hash_idx').on(table.storageHashSha256),
		index('provider_msg_source_idx').on(table.providerMessageId, table.ingestionSourceId),
		index('archived_emails_source_path_idx').on(table.sourcePath),
		index('archived_emails_local_folder_idx').on(table.localFolderId),
		index('archived_emails_duplicate_of_idx').on(table.duplicateOfEmailId),
		index('archived_emails_duplicate_hidden_idx').on(table.isDuplicateHidden),
		index('archived_emails_fuzzy_subject_sender_idx').on(
			table.duplicateSubjectHash,
			table.senderEmail
		),
		index('archived_emails_fuzzy_group_key_idx').on(
			table.isDuplicateHidden,
			table.duplicateFuzzyGroupKey
		),
		index('archived_emails_fuzzy_body_idx').on(table.duplicateBodyHash),
		index('archived_emails_fuzzy_recipients_idx').on(table.duplicateRecipientFingerprint),
		index('archived_emails_fuzzy_attachments_idx').on(table.duplicateAttachmentFingerprint),
		index('archived_emails_remote_content_status_idx').on(table.remoteContentStatus),
	]
);

export const archivedEmailsRelations = relations(archivedEmails, ({ one }) => ({
	ingestionSource: one(ingestionSources, {
		fields: [archivedEmails.ingestionSourceId],
		references: [ingestionSources.id],
	}),
	localFolder: one(archiveFolders, {
		fields: [archivedEmails.localFolderId],
		references: [archiveFolders.id],
	}),
	duplicateOfEmail: one(archivedEmails, {
		fields: [archivedEmails.duplicateOfEmailId],
		references: [archivedEmails.id],
		relationName: 'duplicateReview',
	}),
}));
