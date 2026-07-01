import { api } from '$lib/server/api';
import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import type { ExactDuplicateGroupsResult, FuzzyDuplicateGroupsResult } from '@open-archiver/types';

function getPositiveInteger(value: string | null, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

const ALLOWED_REASONS = new Set([
	'message_id',
	'storage_hash',
	'attachment_hash_set',
	'sender_recipients_sent',
]);

export const load: PageServerLoad = async (event) => {
	const limit = Math.min(getPositiveInteger(event.url.searchParams.get('limit'), 25), 100);
	// Independent pagination per tab.
	const exactPage = getPositiveInteger(event.url.searchParams.get('exactPage'), 1);
	const fuzzyPage = getPositiveInteger(event.url.searchParams.get('fuzzyPage'), 1);
	const reasonParam = event.url.searchParams.get('reason') || '';
	const reason = ALLOWED_REASONS.has(reasonParam) ? reasonParam : '';

	const exactParams = new URLSearchParams({ page: String(exactPage), limit: String(limit) });
	if (reason) exactParams.set('reason', reason);

	const response = await api(`/archived-emails/duplicates/exact?${exactParams.toString()}`, event);
	const body = await response.json();
	if (!response.ok) {
		return error(response.status, body.message || 'Failed to load duplicate groups.');
	}

	const fuzzyResponse = await api(
		`/archived-emails/duplicates/fuzzy?page=${fuzzyPage}&limit=${limit}`,
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
		activeReason: reason,
	};
};
