import { sqliteTable, integer, text } from 'drizzle-orm/sqlite-core';
import type { SystemSettings } from '@open-archiver/types';

export const systemSettings = sqliteTable('system_settings', {
	id: integer('id').primaryKey({ autoIncrement: true }),
	config: text('config', { mode: 'json' }).$type<SystemSettings>().notNull(),
});
