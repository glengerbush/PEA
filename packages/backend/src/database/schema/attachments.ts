import { relations } from 'drizzle-orm';
import { sqliteTable, text, integer, index } from 'drizzle-orm/sqlite-core';
import { randomUUID } from 'crypto';
import { archivedEmails } from './archived-emails';
import { ingestionSources } from './ingestion-sources';

export const attachments = sqliteTable(
	'attachments',
	{
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		filename: text('filename').notNull(),
		mimeType: text('mime_type'),
		sizeBytes: integer('size_bytes', { mode: 'number' }).notNull(),
		contentHashSha256: text('content_hash_sha256').notNull(),
		storagePath: text('storage_path').notNull(),
		ingestionSourceId: text('ingestion_source_id').references(() => ingestionSources.id, {
			onDelete: 'cascade',
		}),
	},
	(table) => [index('source_hash_idx').on(table.ingestionSourceId, table.contentHashSha256)]
);

export const emailAttachments = sqliteTable(
	'email_attachments',
	{
		// Surrogate PK (was a composite PK on email_id+attachment_id). The composite
		// key silently collapsed two byte-identical attachments in the same email to
		// one link — a fidelity loss on reconstruction. A surrogate id lets an email
		// hold multiple links to the same deduplicated attachment record.
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		emailId: text('email_id')
			.notNull()
			.references(() => archivedEmails.id, { onDelete: 'cascade' }),
		attachmentId: text('attachment_id')
			.notNull()
			.references(() => attachments.id, { onDelete: 'restrict' }),
	},
	(t) => ({
		emailIdx: index('email_attachments_email_idx').on(t.emailId),
		attachmentIdx: index('email_attachments_attachment_idx').on(t.attachmentId),
	})
);

export const attachmentsRelations = relations(attachments, ({ many }) => ({
	emailAttachments: many(emailAttachments),
}));

export const emailAttachmentsRelations = relations(emailAttachments, ({ one }) => ({
	archivedEmail: one(archivedEmails, {
		fields: [emailAttachments.emailId],
		references: [archivedEmails.id],
	}),
	attachment: one(attachments, {
		fields: [emailAttachments.attachmentId],
		references: [attachments.id],
	}),
}));
