import { api } from '$lib/server/api';
import type { PageServerLoad } from './$types';
import type { SafeIngestionSource, IngestionSourceStats } from '@open-archiver/types';
import { error } from '@sveltejs/kit';
export const load: PageServerLoad = async (event) => {
	const response = await api('/ingestion-sources', event);
	const responseText = await response.json();
	if (!response.ok) {
		throw error(response.status, responseText.message || 'Failed to fetch imports.');
	}
	const ingestionSources: SafeIngestionSource[] = responseText;

	// Per-source storage usage (bytes), for the Storage column. Best-effort —
	// a failure just leaves the column showing 0 rather than breaking the page.
	let storageStats: IngestionSourceStats[] = [];
	try {
		const storageRes = await api('/dashboard/ingestion-sources', event);
		if (storageRes.ok) storageStats = await storageRes.json();
	} catch {
		storageStats = [];
	}

	return {
		ingestionSources,
		storageStats,
	};
};
