import { sqliteTable, text, integer } from 'drizzle-orm/sqlite-core';
import { relations, sql } from 'drizzle-orm';
import { randomUUID } from 'crypto';
import { ingestionSources } from './ingestion-sources';

/**
 * Tracks the progress of a single sync cycle (initial import or continuous sync).
 * Used as the coordination layer to replace BullMQ FlowProducer parent/child tracking.
 * Each process-mailbox job atomically increments completed/failed counters here,
 * and the last job to finish dispatches the sync-cycle-finished job.
 */
export const syncSessions = sqliteTable('sync_sessions', {
	id: text('id')
		.primaryKey()
		.$defaultFn(() => randomUUID()),
	ingestionSourceId: text('ingestion_source_id')
		.notNull()
		.references(() => ingestionSources.id, { onDelete: 'cascade' }),
	isInitialImport: integer('is_initial_import', { mode: 'boolean' }).notNull().default(false),
	totalMailboxes: integer('total_mailboxes').notNull().default(0),
	completedMailboxes: integer('completed_mailboxes').notNull().default(0),
	failedMailboxes: integer('failed_mailboxes').notNull().default(0),
	/** Aggregated error messages from all failed process-mailbox jobs (JSON array) */
	errorMessages: text('error_messages', { mode: 'json' })
		.$type<string[]>()
		.notNull()
		.default(sql`'[]'`),
	createdAt: integer('created_at', { mode: 'timestamp_ms' })
		.notNull()
		.default(sql`(unixepoch() * 1000)`),
	/**
	 * Updated each time a process-mailbox job reports its result.
	 * Used to detect genuinely stuck sessions (no activity for N minutes) vs.
	 * large imports that are still actively running.
	 */
	lastActivityAt: integer('last_activity_at', { mode: 'timestamp_ms' })
		.notNull()
		.default(sql`(unixepoch() * 1000)`),
});

export const syncSessionsRelations = relations(syncSessions, ({ one }) => ({
	ingestionSource: one(ingestionSources, {
		fields: [syncSessions.ingestionSourceId],
		references: [ingestionSources.id],
	}),
}));
