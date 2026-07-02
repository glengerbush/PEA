import { sqliteTable, text, integer } from 'drizzle-orm/sqlite-core';
import { relations, sql } from 'drizzle-orm';
import { randomUUID } from 'crypto';
import { ingestionSources } from './ingestion-sources';

export const journalingSourceStatusValues = ['active', 'paused'] as const;

export const journalingSources = sqliteTable('journaling_sources', {
	id: text('id')
		.primaryKey()
		.$defaultFn(() => randomUUID()),
	name: text('name').notNull(),
	/** CIDR blocks or IP addresses allowed to send journal reports */
	allowedIps: text('allowed_ips', { mode: 'json' }).notNull().$type<string[]>(),
	/** Whether to reject non-TLS connections (GDPR compliance) */
	requireTls: integer('require_tls', { mode: 'boolean' }).notNull().default(true),
	/** Optional SMTP AUTH username */
	smtpUsername: text('smtp_username'),
	/** Bcrypt-hashed SMTP AUTH password */
	smtpPasswordHash: text('smtp_password_hash'),
	status: text('status', { enum: journalingSourceStatusValues }).notNull().default('active'),
	/** The backing ingestion source that owns all archived emails */
	ingestionSourceId: text('ingestion_source_id')
		.notNull()
		.references(() => ingestionSources.id, { onDelete: 'cascade' }),
	/** Persisted SMTP routing address generated at creation time (immutable unless regenerated) */
	routingAddress: text('routing_address').notNull(),
	/** Running count of emails received via this journaling endpoint */
	totalReceived: integer('total_received').notNull().default(0),
	/** Timestamp of the last email received */
	lastReceivedAt: integer('last_received_at', { mode: 'timestamp_ms' }),
	createdAt: integer('created_at', { mode: 'timestamp_ms' })
		.notNull()
		.default(sql`(unixepoch() * 1000)`),
	updatedAt: integer('updated_at', { mode: 'timestamp_ms' })
		.notNull()
		.default(sql`(unixepoch() * 1000)`),
});

export const journalingSourcesRelations = relations(journalingSources, ({ one }) => ({
	ingestionSource: one(ingestionSources, {
		fields: [journalingSources.ingestionSourceId],
		references: [ingestionSources.id],
	}),
}));
