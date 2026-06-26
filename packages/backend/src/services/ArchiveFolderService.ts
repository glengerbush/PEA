import { asc, eq, inArray } from 'drizzle-orm';
import { db } from '../database';
import { archivedEmails, archiveFolders } from '../database/schema';
import { AuditService } from './AuditService';
import { SearchService } from './SearchService';
import type { ArchiveFolder, MoveArchivedEmailsResult } from '@open-archiver/types';

type ArchiveFolderRecord = typeof archiveFolders.$inferSelect;

const MAX_BULK_MOVE_SIZE = 1000;
const INDEX_UPDATE_BATCH_SIZE = 500;

function sanitizeFolderSegment(segment: string): string {
	return segment
		.replace(/[\x00-\x1F\x7F]/g, '')
		.replace(/[\\/]+/g, ' ')
		.trim();
}

function normalizeFolderPath(rawPath: string): string {
	const path = rawPath.split('/').map(sanitizeFolderSegment).filter(Boolean).join('/');

	if (!path) {
		throw new Error('Folder path is required');
	}

	if (path.length > 512) {
		throw new Error('Folder path is too long');
	}

	return path;
}

function mapFolder(folder: ArchiveFolderRecord): ArchiveFolder {
	return {
		id: folder.id,
		parentId: folder.parentId,
		name: folder.name,
		path: folder.path,
		createdAt: folder.createdAt,
		updatedAt: folder.updatedAt,
	};
}

export class ArchiveFolderService {
	private static auditService = new AuditService();
	private static searchService = new SearchService();

	public static async listFolders(): Promise<ArchiveFolder[]> {
		const folders = await db.select().from(archiveFolders).orderBy(asc(archiveFolders.path));
		return folders.map(mapFolder);
	}

	public static async createFolder(rawPath: string): Promise<ArchiveFolder> {
		return mapFolder(await this.ensureFolderPath(rawPath));
	}

	public static async moveEmailsToFolder(
		emailIds: string[],
		rawLocalFolderPath: string,
		userId: string,
		actorIp: string
	): Promise<MoveArchivedEmailsResult> {
		const uniqueEmailIds = Array.from(new Set(emailIds.filter(Boolean)));
		if (uniqueEmailIds.length === 0) {
			throw new Error('At least one email must be selected');
		}
		if (uniqueEmailIds.length > MAX_BULK_MOVE_SIZE) {
			throw new Error(`Bulk moves are limited to ${MAX_BULK_MOVE_SIZE} emails at a time`);
		}

		const folder = await this.ensureFolderPath(rawLocalFolderPath);

		const updatedEmails = await db
			.update(archivedEmails)
			.set({
				localFolderId: folder.id,
				localFolderPath: folder.path,
			})
			.where(inArray(archivedEmails.id, uniqueEmailIds))
			.returning({ id: archivedEmails.id });

		for (let i = 0; i < updatedEmails.length; i += INDEX_UPDATE_BATCH_SIZE) {
			const batch = updatedEmails.slice(i, i + INDEX_UPDATE_BATCH_SIZE);
			await this.searchService.updateDocuments(
				'emails',
				batch.map((email) => ({
					id: email.id,
					localFolderId: folder.id,
					localFolderPath: folder.path,
				})),
				'id'
			);
		}

		await this.auditService.createAuditLog({
			actorIdentifier: userId,
			actionType: 'UPDATE',
			targetType: 'ArchivedEmail',
			targetId: 'bulk',
			actorIp,
			details: {
				action: 'MOVE_LOCAL_FOLDER',
				requestedCount: uniqueEmailIds.length,
				movedCount: updatedEmails.length,
				localFolderId: folder.id,
				localFolderPath: folder.path,
			},
		});

		return {
			requestedCount: uniqueEmailIds.length,
			movedCount: updatedEmails.length,
			folder: mapFolder(folder),
		};
	}

	private static async ensureFolderPath(rawPath: string): Promise<ArchiveFolderRecord> {
		const normalizedPath = normalizeFolderPath(rawPath);
		const existing = await db.query.archiveFolders.findFirst({
			where: eq(archiveFolders.path, normalizedPath),
		});
		if (existing) return existing;

		const segments = normalizedPath.split('/');
		let parentId: string | null = null;
		let currentFolder: ArchiveFolderRecord | null = null;

		for (let i = 0; i < segments.length; i += 1) {
			const currentPath = segments.slice(0, i + 1).join('/');
			currentFolder =
				(await db.query.archiveFolders.findFirst({
					where: eq(archiveFolders.path, currentPath),
				})) || null;

			if (!currentFolder) {
				const insertedFolders: ArchiveFolderRecord[] = await db
					.insert(archiveFolders)
					.values({
						parentId,
						name: segments[i],
						path: currentPath,
					})
					.onConflictDoNothing({ target: archiveFolders.path })
					.returning();

				currentFolder =
					insertedFolders[0] ||
					(await db.query.archiveFolders.findFirst({
						where: eq(archiveFolders.path, currentPath),
					})) ||
					null;
			}

			if (!currentFolder) {
				throw new Error(`Failed to create folder ${currentPath}`);
			}

			parentId = currentFolder.id;
		}

		if (!currentFolder) {
			throw new Error(`Failed to create folder ${normalizedPath}`);
		}

		return currentFolder;
	}
}
