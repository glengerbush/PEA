import { relations } from 'drizzle-orm';
import {
	boolean,
	index,
	integer,
	jsonb,
	pgTable,
	primaryKey,
	text,
	timestamp,
	uuid,
} from 'drizzle-orm/pg-core';
import { archivedEmails } from './archived-emails';

export const fuzzyDuplicateGroups = pgTable(
	'fuzzy_duplicate_groups',
	{
		id: uuid('id').primaryKey().defaultRandom(),
		groupKey: text('group_key').notNull().unique(),
		status: text('status').notNull().default('pending'),
		score: integer('score').notNull(),
		signals: jsonb('signals'),
		createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
		updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
	},
	(table) => [
		index('fuzzy_duplicate_groups_status_idx').on(table.status),
		index('fuzzy_duplicate_groups_score_idx').on(table.score),
	]
);

export const fuzzyDuplicateGroupEmails = pgTable(
	'fuzzy_duplicate_group_emails',
	{
		groupId: uuid('group_id')
			.notNull()
			.references(() => fuzzyDuplicateGroups.id, { onDelete: 'cascade' }),
		emailId: uuid('email_id')
			.notNull()
			.references(() => archivedEmails.id, { onDelete: 'cascade' }),
		suggestedKeeper: boolean('suggested_keeper').notNull().default(false),
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
