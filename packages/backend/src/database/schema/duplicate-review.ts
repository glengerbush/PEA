import { relations, sql } from 'drizzle-orm';
import { sqliteTable, text, integer, index, primaryKey } from 'drizzle-orm/sqlite-core';
import { randomUUID } from 'crypto';
import { archivedEmails } from './archived-emails';

export const fuzzyDuplicateGroups = sqliteTable(
	'fuzzy_duplicate_groups',
	{
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		groupKey: text('group_key').notNull().unique(),
		status: text('status').notNull().default('pending'),
		score: integer('score').notNull(),
		signals: text('signals', { mode: 'json' }),
		createdAt: integer('created_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
		updatedAt: integer('updated_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
	},
	(table) => [
		index('fuzzy_duplicate_groups_status_idx').on(table.status),
		index('fuzzy_duplicate_groups_score_idx').on(table.score),
	]
);

export const fuzzyDuplicateGroupEmails = sqliteTable(
	'fuzzy_duplicate_group_emails',
	{
		groupId: text('group_id')
			.notNull()
			.references(() => fuzzyDuplicateGroups.id, { onDelete: 'cascade' }),
		emailId: text('email_id')
			.notNull()
			.references(() => archivedEmails.id, { onDelete: 'cascade' }),
		suggestedKeeper: integer('suggested_keeper', { mode: 'boolean' }).notNull().default(false),
	},
	(table) => [
		primaryKey({ columns: [table.groupId, table.emailId] }),
		index('fuzzy_duplicate_group_emails_email_idx').on(table.emailId),
	]
);

export const fuzzyDuplicateGroupsRelations = relations(fuzzyDuplicateGroups, ({ many }) => ({
	emails: many(fuzzyDuplicateGroupEmails),
}));

export const fuzzyDuplicateGroupEmailsRelations = relations(
	fuzzyDuplicateGroupEmails,
	({ one }) => ({
		group: one(fuzzyDuplicateGroups, {
			fields: [fuzzyDuplicateGroupEmails.groupId],
			references: [fuzzyDuplicateGroups.id],
		}),
		email: one(archivedEmails, {
			fields: [fuzzyDuplicateGroupEmails.emailId],
			references: [archivedEmails.id],
		}),
	})
);
