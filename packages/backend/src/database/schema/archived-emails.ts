import { relations, sql } from 'drizzle-orm';
import { sqliteTable, text, integer, index } from 'drizzle-orm/sqlite-core';
import { randomUUID } from 'crypto';
import { ingestionSources } from './ingestion-sources';

export const archivedEmails = sqliteTable(
	'archived_emails',
	{
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		threadId: text('thread_id'),
		ingestionSourceId: text('ingestion_source_id')
			.notNull()
			.references(() => ingestionSources.id, { onDelete: 'cascade' }),
		userEmail: text('user_email').notNull(),
		messageIdHeader: text('message_id_header'),
		/** The provider-specific message ID (e.g., Gmail API ID, Graph API ID).
		 * Used by the pre-fetch duplicate check to avoid unnecessary API calls during retries. */
		providerMessageId: text('provider_message_id'),
		sentAt: integer('sent_at', { mode: 'timestamp_ms' }).notNull(),
		subject: text('subject'),
		senderName: text('sender_name'),
		senderEmail: text('sender_email').notNull(),
		recipients: text('recipients', { mode: 'json' }),
		storagePath: text('storage_path').notNull(),
		storageHashSha256: text('storage_hash_sha256').notNull(),
		sizeBytes: integer('size_bytes', { mode: 'number' }).notNull(),
		isIndexed: integer('is_indexed', { mode: 'boolean' }).notNull().default(false),
		hasAttachments: integer('has_attachments', { mode: 'boolean' }).notNull().default(false),
		archivedAt: integer('archived_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
		sourcePath: text('source_path'),
		sourceLabels: text('source_labels', { mode: 'json' }),
		duplicateSubjectHash: text('duplicate_subject_hash'),
		duplicateFuzzyGroupKey: text('duplicate_fuzzy_group_key'),
		duplicateBodyHash: text('duplicate_body_hash'),
		duplicateRecipientFingerprint: text('duplicate_recipient_fingerprint'),
		duplicateAttachmentFingerprint: text('duplicate_attachment_fingerprint'),
		remoteContentStatus: text('remote_content_status').notNull().default('not_started'),
		remoteContentAssetCount: integer('remote_content_asset_count', { mode: 'number' })
			.notNull()
			.default(0),
		remoteContentArchivedAt: integer('remote_content_archived_at', { mode: 'timestamp_ms' }),
		path: text('path'),
		tags: text('tags', { mode: 'json' }),
	},
	(table) => [
		index('thread_id_idx').on(table.threadId),
		index('archived_emails_message_id_header_idx').on(table.messageIdHeader),
		index('archived_emails_storage_hash_idx').on(table.storageHashSha256),
		index('provider_msg_source_idx').on(table.providerMessageId, table.ingestionSourceId),
		index('archived_emails_source_path_idx').on(table.sourcePath),
		index('archived_emails_fuzzy_subject_sender_idx').on(
			table.duplicateSubjectHash,
			table.senderEmail
		),
		index('archived_emails_fuzzy_group_key_idx').on(table.duplicateFuzzyGroupKey),
		index('archived_emails_fuzzy_body_idx').on(table.duplicateBodyHash),
		index('archived_emails_fuzzy_recipients_idx').on(table.duplicateRecipientFingerprint),
		index('archived_emails_fuzzy_attachments_idx').on(table.duplicateAttachmentFingerprint),
		index('archived_emails_remote_content_status_idx').on(table.remoteContentStatus),
		// Supports the mailbox default sort (sent_at desc) without a full scan.
		index('archived_emails_sent_at_idx').on(table.sentAt),
	]
);

export const archivedEmailsRelations = relations(archivedEmails, ({ one }) => ({
	ingestionSource: one(ingestionSources, {
		fields: [archivedEmails.ingestionSourceId],
		references: [ingestionSources.id],
	}),
}));
