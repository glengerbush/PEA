import { sqliteTable, text, integer, index } from 'drizzle-orm/sqlite-core';
import { sql } from 'drizzle-orm';
import { randomUUID } from 'crypto';

/**
 * The in-process job queue (successor to BullMQ→pg-boss). Single-process,
 * single-writer: claiming is race-free because better-sqlite3 is synchronous.
 * Crash recovery = reset 'active' rows to 'pending' at boot.
 */
export const jobs = sqliteTable(
	'jobs',
	{
		id: text('id')
			.primaryKey()
			.$defaultFn(() => randomUUID()),
		/** Queue (worker pool) this job belongs to: ingestion | indexing | remote-content */
		queue: text('queue').notNull(),
		/** Job name dispatched on by the queue's processor map */
		name: text('name').notNull(),
		payload: text('payload', { mode: 'json' }).notNull(),
		state: text('state', { enum: ['pending', 'active', 'completed', 'failed'] })
			.notNull()
			.default('pending'),
		attempts: integer('attempts').notNull().default(0),
		/** Total tries allowed (retryLimit + 1). Masters are 1 — never re-run. */
		maxAttempts: integer('max_attempts').notNull().default(5),
		/** Not claimable before this time (backoff / delayed jobs). */
		runAt: integer('run_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
		/** Suppresses duplicate sends while a job with the same key is pending/active. */
		singletonKey: text('singleton_key'),
		error: text('error'),
		createdAt: integer('created_at', { mode: 'timestamp_ms' })
			.notNull()
			.default(sql`(unixepoch() * 1000)`),
		startedAt: integer('started_at', { mode: 'timestamp_ms' }),
		finishedAt: integer('finished_at', { mode: 'timestamp_ms' }),
	},
	(table) => [
		index('jobs_claim_idx').on(table.queue, table.state, table.runAt),
		index('jobs_singleton_idx').on(table.queue, table.singletonKey, table.state),
		index('jobs_created_idx').on(table.createdAt),
	]
);
