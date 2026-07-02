import { sqliteTable, text, integer, uniqueIndex } from 'drizzle-orm/sqlite-core';
import { sql } from 'drizzle-orm';
import { randomUUID } from 'crypto';

export const contacts = sqliteTable(
	'contacts',
	{
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		/** Lowercased email address — the lookup key. */
		email: text('email').notNull(),
		displayName: text('display_name').notNull(),
		/** Where this contact came from, e.g. 'csv' | 'vcf'. */
		source: text('source'),
		createdAt: integer('created_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
		updatedAt: integer('updated_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
	},
	(table) => [uniqueIndex('contacts_email_idx').on(table.email)]
);
