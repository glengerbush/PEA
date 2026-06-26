import { relations } from 'drizzle-orm';
import { index, pgTable, text, timestamp, uuid, type AnyPgColumn } from 'drizzle-orm/pg-core';

export const archiveFolders = pgTable(
	'archive_folders',
	{
		id: uuid('id').primaryKey().defaultRandom(),
		parentId: uuid('parent_id').references((): AnyPgColumn => archiveFolders.id, {
			onDelete: 'set null',
		}),
		name: text('name').notNull(),
		path: text('path').notNull().unique(),
		createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
		updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
	},
	(table) => [index('archive_folders_parent_idx').on(table.parentId)]
);

export const archiveFoldersRelations = relations(archiveFolders, ({ one, many }) => ({
	parent: one(archiveFolders, {
		fields: [archiveFolders.parentId],
		references: [archiveFolders.id],
		relationName: 'folderChildren',
	}),
	children: many(archiveFolders, {
		relationName: 'folderChildren',
	}),
}));
