import { Index, MeiliSearch, SearchParams } from 'meilisearch';
import { config } from '../config';
import type {
	ArchiveQuery,
	ArchiveQueryFilters,
	ArchiveSearchField,
	ArchiveSortField,
	SearchQuery,
	SearchResult,
	EmailDocument,
	TopSender,
	SortDirection,
} from '@open-archiver/types';
import { IngestionService } from './IngestionService';

type EmailSearchParams = SearchParams & {
	attributesToSearchOn?: string[];
};

const DEFAULT_SEARCH_FIELDS: ArchiveSearchField[] = [
	'subject',
	'body',
	'from',
	'senderName',
	'to',
	'cc',
	'bcc',
	'attachments.filename',
	'attachments.content',
	'userEmail',
	'sourcePath',
	'sourceLabels',
	'tags',
];

const SORT_FIELD_MAP: Record<ArchiveSortField, string> = {
	sentAt: 'timestamp',
	archivedAt: 'archivedAt',
	sender: 'from',
	subject: 'subject',
	sizeBytes: 'sizeBytes',
};

const FILTERABLE_FALLBACK_FIELDS = new Set([
	'from',
	'senderName',
	'to',
	'cc',
	'bcc',
	'timestamp',
	'archivedAt',
	'ingestionSourceId',
	'userEmail',
	'hasAttachments',
	'sourcePath',
	'sourceLabels',
	'tags',
	'threadId',
	'messageIdHeader',
	'sizeBytes',
]);

function clampPositiveInteger(value: number | undefined, fallback: number, max: number): number {
	if (!value || !Number.isFinite(value) || value < 1) {
		return fallback;
	}
	return Math.min(Math.floor(value), max);
}

function quoteFilterValue(value: string): string {
	return JSON.stringify(value);
}

function toTimestamp(value: string | number | Date | undefined): number | null {
	if (value === undefined) return null;
	if (value instanceof Date) return Number.isNaN(value.getTime()) ? null : value.getTime();
	if (typeof value === 'number') return Number.isFinite(value) ? value : null;

	const parsed = Date.parse(value);
	return Number.isNaN(parsed) ? null : parsed;
}

function normalizeSearchFields(fields: ArchiveSearchField[] | undefined): ArchiveSearchField[] {
	if (!fields || fields.length === 0) {
		return DEFAULT_SEARCH_FIELDS;
	}

	const allowed = new Set(DEFAULT_SEARCH_FIELDS);
	const normalized = fields.filter((field) => allowed.has(field));
	return normalized.length > 0 ? normalized : DEFAULT_SEARCH_FIELDS;
}

function normalizeSort(sort: ArchiveSortField | undefined): string {
	return SORT_FIELD_MAP[sort || 'sentAt'];
}

function normalizeDirection(direction: SortDirection | undefined): SortDirection {
	return direction === 'asc' ? 'asc' : 'desc';
}

export class SearchService {
	private client: MeiliSearch;

	constructor() {
		this.client = new MeiliSearch({
			host: config.search.host,
			apiKey: config.search.apiKey,
		});
	}

	public async getIndex<T extends Record<string, any>>(name: string): Promise<Index<T>> {
		return this.client.index<T>(name);
	}

	public async addDocuments<T extends Record<string, any>>(
		indexName: string,
		documents: T[],
		primaryKey?: string
	) {
		const index = await this.getIndex<T>(indexName);
		if (primaryKey) {
			index.update({ primaryKey });
		}
		return index.addDocuments(documents);
	}

	public async updateDocuments<T extends Record<string, any>>(
		indexName: string,
		documents: T[],
		primaryKey?: string
	) {
		const index = await this.getIndex<T>(indexName);
		if (primaryKey) {
			index.update({ primaryKey });
		}
		return index.updateDocuments(documents);
	}

	public async search<T extends Record<string, any>>(
		indexName: string,
		query: string,
		options?: any
	) {
		const index = await this.getIndex<T>(indexName);
		return index.search(query, options);
	}

	public async deleteDocuments(indexName: string, ids: string[]) {
		const index = await this.getIndex(indexName);
		return index.deleteDocuments(ids);
	}

	public async deleteDocumentsByFilter(indexName: string, filter: string | string[]) {
		const index = await this.getIndex(indexName);
		return index.deleteDocuments({ filter });
	}

	public async searchEmails(
		dto: SearchQuery,
		userId: string,
		actorIp: string
	): Promise<SearchResult> {
		return this.queryArchivedEmails(
			{
				query: dto.query,
				filters: dto.filters as ArchiveQueryFilters,
				fields: dto.fields,
				sort: dto.sort,
				direction: dto.direction,
				page: dto.page,
				limit: dto.limit,
				matchingStrategy: dto.matchingStrategy,
			},
			userId,
			actorIp
		);
	}

	public async queryArchivedEmails(
		dto: ArchiveQuery,
		userId: string,
		actorIp: string
	): Promise<SearchResult> {
		const query = dto.query || '';
		const page = clampPositiveInteger(dto.page, 1, Number.MAX_SAFE_INTEGER);
		const limit = clampPositiveInteger(dto.limit, 10, 100);
		const matchingStrategy = dto.matchingStrategy || 'last';
		const fields = normalizeSearchFields(dto.fields);
		const sortField = normalizeSort(dto.sort);
		const direction = normalizeDirection(dto.direction);
		const index = await this.getIndex<EmailDocument>('emails');

		const searchParams: EmailSearchParams = {
			limit,
			offset: (page - 1) * limit,
			attributesToHighlight: fields,
			showMatchesPosition: true,
			sort: [`${sortField}:${direction}`],
			matchingStrategy,
		};

		if (query) {
			searchParams.attributesToSearchOn = fields;
		}

		const filterParts = await this.buildArchiveFilterParts(dto.filters);
		if (filterParts.length > 0) {
			searchParams.filter = filterParts.join(' AND ');
		}

		// console.log('searchParams', searchParams);
		const searchResults = await index.search(query, searchParams);

		if (query) {
		}

		return {
			hits: searchResults.hits,
			total: searchResults.estimatedTotalHits ?? searchResults.hits.length,
			page,
			limit,
			totalPages: Math.ceil(
				(searchResults.estimatedTotalHits ?? searchResults.hits.length) / limit
			),
			processingTimeMs: searchResults.processingTimeMs,
		};
	}

	private async buildArchiveFilterParts(
		filters: ArchiveQueryFilters | undefined
	): Promise<string[]> {
		if (!filters) {
			return [];
		}

		const filterParts: string[] = [];
		const handled = new Set<string>();

		if (filters.ingestionSourceId) {
			const groupIds = await IngestionService.findGroupSourceIds(filters.ingestionSourceId);
			if (groupIds.length === 1) {
				filterParts.push(`ingestionSourceId = ${quoteFilterValue(groupIds[0])}`);
			} else {
				const inList = groupIds.map(quoteFilterValue).join(', ');
				filterParts.push(`ingestionSourceId IN [${inList}]`);
			}
			handled.add('ingestionSourceId');
		}

		for (const key of [
			'userEmail',
			'from',
			'to',
			'cc',
			'bcc',
			'sourcePath',
		] as const) {
			const value = filters[key];
			if (typeof value === 'string' && value.length > 0) {
				filterParts.push(`${key} = ${quoteFilterValue(value)}`);
				handled.add(key);
			}
		}

		if (typeof filters.hasAttachments === 'boolean') {
			filterParts.push(`hasAttachments = ${filters.hasAttachments}`);
			handled.add('hasAttachments');
		}

		if (Array.isArray(filters.sourceLabels)) {
			for (const label of filters.sourceLabels) {
				if (typeof label === 'string' && label.length > 0) {
					filterParts.push(`sourceLabels = ${quoteFilterValue(label)}`);
				}
			}
			handled.add('sourceLabels');
		}

		if (Array.isArray(filters.tags)) {
			for (const tag of filters.tags) {
				if (typeof tag === 'string' && tag.length > 0) {
					filterParts.push(`tags = ${quoteFilterValue(tag)}`);
				}
			}
			handled.add('tags');
		}

		const sentAfter = toTimestamp(filters.sentAfter);
		if (sentAfter !== null) {
			filterParts.push(`timestamp >= ${sentAfter}`);
			handled.add('sentAfter');
		}

		const sentBefore = toTimestamp(filters.sentBefore);
		if (sentBefore !== null) {
			filterParts.push(`timestamp <= ${sentBefore}`);
			handled.add('sentBefore');
		}

		const archivedAfter = toTimestamp(filters.archivedAfter);
		if (archivedAfter !== null) {
			filterParts.push(`archivedAt >= ${archivedAfter}`);
			handled.add('archivedAfter');
		}

		const archivedBefore = toTimestamp(filters.archivedBefore);
		if (archivedBefore !== null) {
			filterParts.push(`archivedAt <= ${archivedBefore}`);
			handled.add('archivedBefore');
		}

		for (const [key, value] of Object.entries(filters)) {
			if (handled.has(key) || !FILTERABLE_FALLBACK_FIELDS.has(key)) {
				continue;
			}

			if (typeof value === 'string' && value.length > 0) {
				filterParts.push(`${key} = ${quoteFilterValue(value)}`);
			} else if (typeof value === 'number' && Number.isFinite(value)) {
				filterParts.push(`${key} = ${value}`);
			} else if (typeof value === 'boolean') {
				filterParts.push(`${key} = ${value}`);
			}
		}

		return filterParts;
	}

	public async getTopSenders(limit = 10): Promise<TopSender[]> {
		const index = await this.getIndex<EmailDocument>('emails');
		const searchResults = await index.search('', {
			facets: ['from'],
			limit: 0,
		});

		if (!searchResults.facetDistribution?.from) {
			return [];
		}

		// Sort and take top N
		const sortedSenders = Object.entries(searchResults.facetDistribution.from)
			.sort(([, countA], [, countB]) => countB - countA)
			.slice(0, limit)
			.map(([sender, count]) => ({ sender, count }));

		return sortedSenders;
	}

	/**
	 * Returns the distinct values present in the archive for the filterable
	 * facets used by the mailbox filter dropdowns (imported folder = sourcePath,
	 * and tags). Values are sorted alphabetically. Note: Meilisearch caps facet
	 * values at `faceting.maxValuesPerFacet` (default 100).
	 */
	public async getFilterFacets(): Promise<{ tags: string[] }> {
		const index = await this.getIndex<EmailDocument>('emails');
		const searchResults = await index.search('', {
			facets: ['tags'],
			limit: 0,
		});
		const distribution = searchResults.facetDistribution || {};
		const distinctSorted = (values?: Record<string, number>): string[] =>
			Object.keys(values || {})
				.filter((value) => value.trim().length > 0)
				.sort((a, b) => a.localeCompare(b));
		return {
			tags: distinctSorted(distribution.tags),
		};
	}

	public async configureEmailIndex() {
		const index = await this.getIndex('emails');
		await index.updateSettings({
			searchableAttributes: [
				'subject',
				'body',
				'from',
				'senderName',
				'to',
				'cc',
				'bcc',
				'attachments.filename',
				'attachments.content',
				'userEmail',
				'sourcePath',
				'sourceLabels',
				'tags',
			],
			filterableAttributes: [
				'from',
				'senderName',
				'to',
				'cc',
				'bcc',
				'timestamp',
				'archivedAt',
				'ingestionSourceId',
				'userEmail',
				'hasAttachments',
				'sourcePath',
				'sourceLabels',
				'tags',
				'threadId',
				'messageIdHeader',
				'sizeBytes',
			],
			sortableAttributes: ['timestamp', 'archivedAt', 'from', 'subject', 'sizeBytes'],
			pagination: {
				maxTotalHits: 1_000_000,
			},
		});
	}
}
