import { relations } from 'drizzle-orm';
import { bigint, index, pgTable, text, timestamp, unique, uuid } from 'drizzle-orm/pg-core';
import { archivedEmails } from './archived-emails';

export const remoteContentAssets = pgTable(
	'remote_content_assets',
	{
		id: uuid('id').primaryKey().defaultRandom(),
		emailId: uuid('email_id')
			.notNull()
			.references(() => archivedEmails.id, { onDelete: 'cascade' }),
		originalUrl: text('original_url').notNull(),
		finalUrl: text('final_url'),
		urlHash: text('url_hash').notNull(),
		status: text('status').notNull().default('pending'),
		contentType: text('content_type'),
		sizeBytes: bigint('size_bytes', { mode: 'number' }),
		contentHashSha256: text('content_hash_sha256'),
		storagePath: text('storage_path'),
		failureReason: text('failure_reason'),
		createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
		updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
	},
	(table) => [
		unique('remote_content_assets_email_url_hash_unique').on(table.emailId, table.urlHash),
		index('remote_content_assets_email_idx').on(table.emailId),
		index('remote_content_assets_status_idx').on(table.status),
	]
);

export const remoteContentAssetsRelations = relations(remoteContentAssets, ({ one }) => ({
	email: one(archivedEmails, {
		fields: [remoteContentAssets.emailId],
		references: [archivedEmails.id],
	}),
}));
