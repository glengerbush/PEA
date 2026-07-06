import { api } from '$lib/api.load';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import type { SearchResult } from '@pea/types';

function getPositiveInteger(value: string | null, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

export const load: PageLoad = async (event) => {
	const page = getPositiveInteger(event.url.searchParams.get('page'), 1);
	const limit = Math.min(getPositiveInteger(event.url.searchParams.get('limit'), 25), 100);

	const params = new URLSearchParams({
		trashed: 'true',
		page: String(page),
		limit: String(limit),
		sort: 'archivedAt',
		direction: 'desc',
	});

	const response = await api(`/archived-emails?${params.toString()}`, event);
	const body = await response.json();
	if (!response.ok) {
		return error(response.status, body.message || 'Failed to load the trash.');
	}

	return { trash: body as SearchResult };
};
