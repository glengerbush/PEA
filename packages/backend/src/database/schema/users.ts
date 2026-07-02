import { sqliteTable, text, integer } from 'drizzle-orm/sqlite-core';
import { sql } from 'drizzle-orm';
import { randomUUID } from 'crypto';

/**
 * The `users` table stores the core user information for authentication and identification.
 */
export const users = sqliteTable('users', {
	id: text('id')
		.primaryKey()
		.$defaultFn(() => randomUUID()),
	email: text('email').notNull().unique(),
	first_name: text('first_name'),
	last_name: text('last_name'),
	password: text('password'),
	provider: text('provider').default('local'),
	providerId: text('provider_id'),
	createdAt: integer('created_at', { mode: 'timestamp_ms' })
		.default(sql`(unixepoch() * 1000)`)
		.notNull(),
	updatedAt: integer('updated_at', { mode: 'timestamp_ms' })
		.default(sql`(unixepoch() * 1000)`)
		.notNull(),
});
