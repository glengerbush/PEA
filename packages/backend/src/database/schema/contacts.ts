import { pgTable, text, timestamp, uuid, uniqueIndex } from 'drizzle-orm/pg-core';

export const contacts = pgTable(
	'contacts',
	{
		id: uuid('id').primaryKey().defaultRandom(),
		/** Lowercased email address — the lookup key. */
		email: text('email').notNull(),
		displayName: text('display_name').notNull(),
		/** Where this contact came from, e.g. 'csv' | 'vcf'. */
		source: text('source'),
		createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
		updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
	},
	(table) => [uniqueIndex('contacts_email_idx').on(table.email)]
);
