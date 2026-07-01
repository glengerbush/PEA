import { api } from '$lib/server/api';
import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import type {
	ArchiveSearchField,
	ArchiveSortField,
	IngestionSource,
	MatchingStrategy,
	SearchResult,
	SortDirection,
} from '@open-archiver/types';

const SORT_FIELDS = new Set<ArchiveSortField>([
	'sentAt',
	'archivedAt',
	'sender',
	'subject',
	'sizeBytes',
]);
const DIRECTIONS = new Set<SortDirection>(['asc', 'desc']);
const MATCHING_STRATEGIES = new Set<MatchingStrategy>(['last', 'all', 'frequency']);
const SEARCH_FIELDS = new Set<ArchiveSearchField>([
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
]);

function getPositiveInteger(value: string | null, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

function getSort(value: string | null): ArchiveSortField {
	return value && SORT_FIELDS.has(value as ArchiveSortField)
		? (value as ArchiveSortField)
		: 'sentAt';
}

function getDirection(value: string | null): SortDirection {
	return value && DIRECTIONS.has(value as SortDirection) ? (value as SortDirection) : 'desc';
}

function getMatchingStrategy(value: string | null): MatchingStrategy {
	return value && MATCHING_STRATEGIES.has(value as MatchingStrategy)
		? (value as MatchingStrategy)
		: 'last';
}

function getFields(value: string | null): ArchiveSearchField[] {
	if (!value || value === 'all') return [];
	return value
		.split(',')
		.map((field) => field.trim())
		.filter((field): field is ArchiveSearchField =>
			SEARCH_FIELDS.has(field as ArchiveSearchField)
		);
}

export const load: PageServerLoad = async (event) => {
	const { url } = event;
	const q = url.searchParams.get('q') || '';
	const fields = getFields(url.searchParams.get('fields'));
	const ingestionSourceId = url.searchParams.get('ingestionSourceId') || 'all';
	const hasAttachments = url.searchParams.get('hasAttachments') || 'any';
	const tags = url.searchParams.get('tags') || '';
	const page = getPositiveInteger(url.searchParams.get('page'), 1);
	const limit = Math.min(getPositiveInteger(url.searchParams.get('limit'), 25), 100);
	const sort = getSort(url.searchParams.get('sort'));
	const direction = getDirection(url.searchParams.get('direction'));
	const matchingStrategy = getMatchingStrategy(url.searchParams.get('matchingStrategy'));

	const sourcesResponse = await api('/ingestion-sources', event);
	const sourcesResponseText = await sourcesResponse.json();
	let ingestionSources: IngestionSource[] = sourcesResponseText;
	if (!sourcesResponse.ok) {
		if (sourcesResponse.status === 403) {
			ingestionSources = [];
		} else {
			return error(
				sourcesResponse.status,
				sourcesResponseText.message || 'Failed to load import source.'
			);
		}
	}

	const facetsResponse = await api('/archived-emails/facets', event);
	let filterFacets: { tags: string[] } = { tags: [] };
	if (facetsResponse.ok) {
		filterFacets = await facetsResponse.json();
	} else if (facetsResponse.status !== 403) {
		const facetsBody = await facetsResponse.json();
		return error(facetsResponse.status, facetsBody.message || 'Failed to load filter options.');
	}

	const archiveParams = new URLSearchParams({
		page: page.toString(),
		limit: limit.toString(),
		sort,
		direction,
		matchingStrategy,
	});

	if (q) archiveParams.set('q', q);
	if (fields.length > 0) archiveParams.set('fields', fields.join(','));
	if (ingestionSourceId !== 'all') archiveParams.set('ingestionSourceId', ingestionSourceId);
	if (hasAttachments === 'true' || hasAttachments === 'false') {
		archiveParams.set('hasAttachments', hasAttachments);
	}
	if (tags) archiveParams.set('tags', tags);

	const emptySearchResult: SearchResult = {
		hits: [],
		total: 0,
		page,
		limit,
		totalPages: 0,
		processingTimeMs: 0,
	};

	const emailsResponse = await api(`/archived-emails?${archiveParams.toString()}`, event);
	const emailsResponseBody = await emailsResponse.json();
	if (!emailsResponse.ok) {
		return error(
			emailsResponse.status,
			emailsResponseBody.message || 'Failed to load archived emails.'
		);
	}

	return {
		ingestionSources,
		filterFacets,
		searchResult: (emailsResponseBody as SearchResult) || emptySearchResult,
		filters: {
			q,
			fields: fields.join(',') || 'all',
			ingestionSourceId,
			hasAttachments,
			tags,
			page,
			limit,
			sort,
			direction,
			matchingStrategy,
		},
	};
};
