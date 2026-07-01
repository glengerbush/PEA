import { eq, inArray } from 'drizzle-orm';
import { db } from '../database';
import { archivedEmails } from '../database/schema';
import { SearchService } from './SearchService';
import type {
	UpdateArchivedEmailTagsDto,
	UpdateArchivedEmailTagsResult,
	UpdatedArchivedEmailTags,
} from '@open-archiver/types';

const MAX_BULK_TAG_SIZE = 1000;
const MAX_TAGS_PER_EMAIL = 64;
const MAX_TAG_LENGTH = 64;
const INDEX_UPDATE_BATCH_SIZE = 500;

function normalizeTag(rawTag: string): string {
	return rawTag
		.replace(/[\x00-\x1F\x7F]/g, '')
		.replace(/\s+/g, ' ')
		.trim()
		.replace(/^#+/, '')
		.trim()
		.slice(0, MAX_TAG_LENGTH);
}

function normalizeTags(rawTags: unknown): string[] {
	const source =
		typeof rawTags === 'string' ? rawTags.split(',') : Array.isArray(rawTags) ? rawTags : [];
	const tags: string[] = [];
	const seen = new Set<string>();

	for (const rawTag of source) {
		if (typeof rawTag !== 'string') continue;
		const tag = normalizeTag(rawTag);
		const key = tag.toLocaleLowerCase();
		if (!tag || seen.has(key)) continue;
		seen.add(key);
		tags.push(tag);
	}

	return tags;
}

function tagsEqual(left: string[], right: string[]): boolean {
	return left.length === right.length && left.every((tag, index) => tag === right[index]);
}

function applyTagChanges(currentTags: unknown, addTags: string[], removeTags: string[]): string[] {
	const removeKeys = new Set(removeTags.map((tag) => tag.toLocaleLowerCase()));
	const nextTags: string[] = [];
	const seen = new Set<string>();

	for (const tag of normalizeTags(currentTags)) {
		const key = tag.toLocaleLowerCase();
		if (removeKeys.has(key) || seen.has(key)) continue;
		seen.add(key);
		nextTags.push(tag);
	}

	for (const tag of addTags) {
		const key = tag.toLocaleLowerCase();
		if (removeKeys.has(key) || seen.has(key)) continue;
		seen.add(key);
		nextTags.push(tag);
		if (nextTags.length >= MAX_TAGS_PER_EMAIL) break;
	}

	return nextTags;
}

export class ArchiveTagService {
	private static searchService = new SearchService();

	public static async updateEmailTags(
		dto: UpdateArchivedEmailTagsDto,
		userId: string,
		actorIp: string
	): Promise<UpdateArchivedEmailTagsResult> {
		const emailIds: string[] = Array.from(
			new Set<string>(
				dto.emailIds.filter(
					(emailId): emailId is string =>
						typeof emailId === 'string' && emailId.length > 0
				)
			)
		);
		if (emailIds.length === 0) {
			throw new Error('At least one email must be selected');
		}
		if (emailIds.length > MAX_BULK_TAG_SIZE) {
			throw new Error(
				`Bulk tag updates are limited to ${MAX_BULK_TAG_SIZE} emails at a time`
			);
		}

		const addedTags = normalizeTags(dto.addTags);
		const removedTags = normalizeTags(dto.removeTags);
		if (addedTags.length === 0 && removedTags.length === 0) {
			throw new Error('At least one tag must be added or removed');
		}

		const rows = await db
			.select({
				id: archivedEmails.id,
				tags: archivedEmails.tags,
			})
			.from(archivedEmails)
			.where(inArray(archivedEmails.id, emailIds));

		const updates: UpdatedArchivedEmailTags[] = [];
		for (const row of rows) {
			const currentTags = normalizeTags(row.tags);
			const tags = applyTagChanges(currentTags, addedTags, removedTags);
			if (tagsEqual(currentTags, tags)) continue;

			await db.update(archivedEmails).set({ tags }).where(eq(archivedEmails.id, row.id));
			updates.push({ id: row.id, tags });
		}

		for (let i = 0; i < updates.length; i += INDEX_UPDATE_BATCH_SIZE) {
			const batch = updates.slice(i, i + INDEX_UPDATE_BATCH_SIZE);
			await this.searchService.updateDocuments(
				'emails',
				batch.map((email) => ({
					id: email.id,
					tags: email.tags,
				})),
				'id'
			);
		}


		return {
			requestedCount: emailIds.length,
			updatedCount: updates.length,
			addedTags,
			removedTags,
			emails: updates,
		};
	}
}
