import { api } from '$lib/api.load';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import type { DuplicateClassification, DuplicateGroupsResult } from '@pea/types';

function getPositiveInteger(value: string | null, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

const ALLOWED_CLASSIFICATIONS = new Set<DuplicateClassification>(['exact', 'likely']);

export const load: PageLoad = async (event) => {
	const limit = Math.min(getPositiveInteger(event.url.searchParams.get('limit'), 25), 100);
	const exactPage = getPositiveInteger(event.url.searchParams.get('exactPage'), 1);
	const classificationParam = event.url.searchParams.get('classification') || 'exact';
	const classification = ALLOWED_CLASSIFICATIONS.has(
		classificationParam as DuplicateClassification
	)
		? (classificationParam as DuplicateClassification)
		: 'exact';

	const exactParams = new URLSearchParams({ page: String(exactPage), limit: String(limit) });
	if (classification) exactParams.set('classification', classification);

	const response = await api(
		`/archived-emails/duplicates/exact?${exactParams.toString()}`,
		event
	);
	const body = await response.json();
	if (!response.ok) {
		return error(response.status, body.message || 'Failed to load duplicate groups.');
	}

	return {
		duplicateGroups: body as DuplicateGroupsResult,
		activeClassification: classification,
	};
};
