import { relations, sql } from 'drizzle-orm';
import { sqliteTable, text, integer, index, unique } from 'drizzle-orm/sqlite-core';
import { randomUUID } from 'crypto';
import { archivedEmails } from './archived-emails';

export const remoteContentAssets = sqliteTable(
	'remote_content_assets',
	{
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		emailId: text('email_id')
			.notNull()
			.references(() => archivedEmails.id, { onDelete: 'cascade' }),
		originalUrl: text('original_url').notNull(),
		finalUrl: text('final_url'),
		urlHash: text('url_hash').notNull(),
		status: text('status').notNull().default('pending'),
		contentType: text('content_type'),
		sizeBytes: integer('size_bytes', { mode: 'number' }),
		contentHashSha256: text('content_hash_sha256'),
		storagePath: text('storage_path'),
		failureReason: text('failure_reason'),
		createdAt: integer('created_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
		updatedAt: integer('updated_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
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
