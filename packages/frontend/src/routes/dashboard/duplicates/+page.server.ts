import { api } from '$lib/server/api';
import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import type { ExactDuplicateGroupsResult, FuzzyDuplicateGroupsResult } from '@open-archiver/types';

function getPositiveInteger(value: string | null, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

export const load: PageServerLoad = async (event) => {
	const page = getPositiveInteger(event.url.searchParams.get('page'), 1);
	const limit = Math.min(getPositiveInteger(event.url.searchParams.get('limit'), 25), 100);

	const response = await api(
		`/archived-emails/duplicates/exact?page=${page}&limit=${limit}`,
		event
	);
	const body = await response.json();
	if (!response.ok) {
		return error(response.status, body.message || 'Failed to load duplicate groups.');
	}

	const fuzzyResponse = await api(
		`/archived-emails/duplicates/fuzzy?page=${page}&limit=${limit}`,
		event
	);
	const fuzzyBody = await fuzzyResponse.json();
	if (!fuzzyResponse.ok) {
		return error(
			fuzzyResponse.status,
			fuzzyBody.message || 'Failed to load fuzzy duplicate groups.'
		);
	}

	return {
		duplicateGroups: body as ExactDuplicateGroupsResult,
		fuzzyDuplicateGroups: fuzzyBody as FuzzyDuplicateGroupsResult,
	};
};
