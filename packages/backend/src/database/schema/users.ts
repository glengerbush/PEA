import { pgTable, text, timestamp, uuid } from 'drizzle-orm/pg-core';

/**
 * The `users` table stores the core user information for authentication and identification.
 */
export const users = pgTable('users', {
	id: uuid('id').primaryKey().defaultRandom(),
	email: text('email').notNull().unique(),
	first_name: text('first_name'),
	last_name: text('last_name'),
	password: text('password'),
	provider: text('provider').default('local'),
	providerId: text('provider_id'),
	createdAt: timestamp('created_at').defaultNow().notNull(),
	updatedAt: timestamp('updated_at').defaultNow().notNull(),
});

/**
 * The `sessions` table stores user session information for managing login state.
 * It links a session to a user and records its expiration time.
 */
export const sessions = pgTable('sessions', {
	id: text('id').primaryKey(),
	userId: uuid('user_id')
		.notNull()
		.references(() => users.id, { onDelete: 'cascade' }),
	expiresAt: timestamp('expires_at', {
		withTimezone: true,
		mode: 'date',
	}).notNull(),
});
