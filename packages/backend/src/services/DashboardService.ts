import { and, asc, count, desc, eq, gte, inArray, sql } from 'drizzle-orm';
import type {
	IndexedInsights,
	RemoteContentIssue,
	RemoteContentIssueAsset,
	RemoteContentIssuesResult,
} from '@open-archiver/types';

import { archivedEmails, ingestionSources, remoteContentAssets } from '../database/schema';
import { DatabaseService } from './DatabaseService';
import { SearchService } from './SearchService';

class DashboardService {
	#db;
	#searchService;

	constructor(databaseService: DatabaseService, searchService: SearchService) {
		this.#db = databaseService.db;
		this.#searchService = searchService;
	}

	public async getStats() {
		const totalEmailsArchived = await this.#db.select({ count: count() }).from(archivedEmails);
		const totalStorageUsed = await this.#db
			.select({ sum: sql<number>`sum(${archivedEmails.sizeBytes})` })
			.from(archivedEmails);

		const sevenDaysAgo = new Date();
		sevenDaysAgo.setDate(sevenDaysAgo.getDate() - 7);

		const failedIngestionsLast7Days = await this.#db
			.select({ count: count() })
			.from(ingestionSources)
			.where(
				and(
					eq(ingestionSources.status, 'error'),
					gte(ingestionSources.updatedAt, sevenDaysAgo)
				)
			);

		// Remote-content fetch outcomes recorded per email (the batch job itself
		// always succeeds; failures live here, not in the queue).
		const remoteContent = await this.#db
			.select({
				failed: sql<number>`count(*) filter (where ${archivedEmails.remoteContentStatus} = 'failed')`,
				partial: sql<number>`count(*) filter (where ${archivedEmails.remoteContentStatus} = 'partial')`,
			})
			.from(archivedEmails);

		return {
			totalEmailsArchived: totalEmailsArchived[0].count,
			totalStorageUsed: totalStorageUsed[0].sum || 0,
			failedIngestionsLast7Days: failedIngestionsLast7Days[0].count,
			remoteContentFailed: Number(remoteContent[0]?.failed ?? 0),
			remoteContentPartial: Number(remoteContent[0]?.partial ?? 0),
		};
	}

	/**
	 * Emails whose remote-content archiving failed or only partially succeeded,
	 * with the specific asset failures (url + reason) so the cause is visible at
	 * a glance from the dashboard.
	 */
	public async getRemoteContentIssuesPage(opts: {
		page: number;
		limit: number;
		status: 'all' | 'failed' | 'partial';
		sort: 'date' | 'subject' | 'status';
		direction: 'asc' | 'desc';
	}): Promise<RemoteContentIssuesResult> {
		const statuses = opts.status === 'all' ? ['failed', 'partial'] : [opts.status];
		const where = inArray(archivedEmails.remoteContentStatus, statuses);

		const [{ total }] = await this.#db
			.select({ total: count() })
			.from(archivedEmails)
			.where(where);

		const sortColumn =
			opts.sort === 'subject'
				? archivedEmails.subject
				: opts.sort === 'status'
					? archivedEmails.remoteContentStatus
					: archivedEmails.archivedAt;
		const orderBy = opts.direction === 'asc' ? asc(sortColumn) : desc(sortColumn);

		const emails = await this.#db
			.select({
				id: archivedEmails.id,
				subject: archivedEmails.subject,
				senderName: archivedEmails.senderName,
				senderEmail: archivedEmails.senderEmail,
				status: archivedEmails.remoteContentStatus,
				archivedAt: archivedEmails.archivedAt,
			})
			.from(archivedEmails)
			.where(where)
			.orderBy(orderBy)
			.limit(opts.limit)
			.offset((opts.page - 1) * opts.limit);

		const emailIds = emails.map((e) => e.id);
		const assetsByEmail = new Map<string, RemoteContentIssueAsset[]>();
		if (emailIds.length > 0) {
			const assets = await this.#db
				.select({
					emailId: remoteContentAssets.emailId,
					url: remoteContentAssets.originalUrl,
					status: remoteContentAssets.status,
					reason: remoteContentAssets.failureReason,
				})
				.from(remoteContentAssets)
				.where(
					and(
						inArray(remoteContentAssets.emailId, emailIds),
						inArray(remoteContentAssets.status, ['failed', 'blocked'])
					)
				);
			for (const asset of assets) {
				const list = assetsByEmail.get(asset.emailId) ?? [];
				list.push({ url: asset.url, status: asset.status, reason: asset.reason });
				assetsByEmail.set(asset.emailId, list);
			}
		}

		const items: RemoteContentIssue[] = emails.map((e) => ({
			emailId: e.id,
			subject: e.subject || '(no subject)',
			sender: e.senderName || e.senderEmail || 'Unknown sender',
			status: e.status as 'failed' | 'partial',
			archivedAt:
				e.archivedAt instanceof Date ? e.archivedAt.toISOString() : String(e.archivedAt),
			assets: assetsByEmail.get(e.id) ?? [],
		}));

		return { items, total: Number(total), page: opts.page, limit: opts.limit };
	}

	public async getIngestionHistory() {
		const thirtyDaysAgo = new Date();
		thirtyDaysAgo.setDate(thirtyDaysAgo.getDate() - 30);

		const history = await this.#db
			.select({
				date: sql<string>`date_trunc('day', ${archivedEmails.archivedAt})`,
				count: count(),
			})
			.from(archivedEmails)
			.where(gte(archivedEmails.archivedAt, thirtyDaysAgo))
			.groupBy(sql`date_trunc('day', ${archivedEmails.archivedAt})`)
			.orderBy(sql`date_trunc('day', ${archivedEmails.archivedAt})`);

		return { history };
	}

	public async getIngestionSources() {
		const sources = await this.#db
			.select({
				id: ingestionSources.id,
				name: ingestionSources.name,
				provider: ingestionSources.provider,
				status: ingestionSources.status,
				storageUsed: sql<number>`sum(${archivedEmails.sizeBytes})`.mapWith(Number),
			})
			.from(ingestionSources)
			.leftJoin(archivedEmails, eq(ingestionSources.id, archivedEmails.ingestionSourceId))
			.groupBy(ingestionSources.id);

		return sources;
	}

	public async getRecentSyncs() {
		// This is a placeholder as we don't have a sync job table yet.
		return Promise.resolve([]);
	}

	public async getIndexedInsights(): Promise<IndexedInsights> {
		const topSenders = await this.#searchService.getTopSenders(10);
		return {
			topSenders,
		};
	}
}

export const dashboardService = new DashboardService(new DatabaseService(), new SearchService());
