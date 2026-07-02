import { sqliteTable, text, integer, index, type AnySQLiteColumn } from 'drizzle-orm/sqlite-core';
import { relations, sql } from 'drizzle-orm';
import { randomUUID } from 'crypto';
import { users } from './users';

export const ingestionProviderValues = [
	'google_workspace',
	'microsoft_365',
	'generic_imap',
	'pst_import',
	'eml_import',
	'mbox_import',
	'smtp_journaling',
] as const;

export const ingestionStatusValues = [
	'active',
	'paused',
	'error',
	'pending_auth',
	'syncing',
	'importing',
	'auth_success',
	'imported',
	'partially_active',
] as const;

export const ingestionSources = sqliteTable(
	'ingestion_sources',
	{
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		userId: text('user_id').references(() => users.id, { onDelete: 'cascade' }),
		name: text('name').notNull(),
		provider: text('provider', { enum: ingestionProviderValues }).notNull(),
		credentials: text('credentials'),
		status: text('status', { enum: ingestionStatusValues })
			.notNull()
			.default('pending_auth'),
		lastSyncStartedAt: integer('last_sync_started_at', { mode: 'timestamp_ms' }),
		lastSyncFinishedAt: integer('last_sync_finished_at', { mode: 'timestamp_ms' }),
		lastSyncStatusMessage: text('last_sync_status_message'),
		syncState: text('sync_state', { mode: 'json' }),
		/** Self-referencing FK for merge groups. When set, this source is a child
		 *  whose emails are logically grouped with the root source. Flat hierarchy only. */
		mergedIntoId: text('merged_into_id').references(
			(): AnySQLiteColumn => ingestionSources.id,
			{ onDelete: 'set null' }
		),
		createdAt: integer('created_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
		updatedAt: integer('updated_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
	},
	(table) => [index('idx_merged_into').on(table.mergedIntoId)]
);

export const ingestionSourcesRelations = relations(ingestionSources, ({ one, many }) => ({
	user: one(users, {
		fields: [ingestionSources.userId],
		references: [users.id],
	}),
	/** The root source this child is merged into (null if this is a root). */
	mergedInto: one(ingestionSources, {
		fields: [ingestionSources.mergedIntoId],
		references: [ingestionSources.id],
		relationName: 'mergedChildren',
	}),
	/** Child sources that are merged into this root. */
	children: many(ingestionSources, {
		relationName: 'mergedChildren',
	}),
}));
