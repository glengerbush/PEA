import { api } from '$lib/api.load';
import { decodeMailboxSearchRequest } from '$lib/mailbox-search-state';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import type { IngestionSource, SearchResult } from '@pea/types';

export const load: PageLoad = async (event) => {
	const { url } = event;
	const { intent: searchIntent, params: archiveParams } = decodeMailboxSearchRequest(url);

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

	const emptySearchResult: SearchResult = {
		hits: [],
		total: 0,
		page: searchIntent.page,
		limit: searchIntent.limit,
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
		searchIntent,
	};
};
