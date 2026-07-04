import { api } from '$lib/api.load';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import type { ExactDuplicateGroupsResult, LikelyDuplicateGroupsResult } from '@pea/types';

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

export const load: PageLoad = async (event) => {
	const limit = Math.min(getPositiveInteger(event.url.searchParams.get('limit'), 25), 100);
	// Independent pagination per tab.
	const exactPage = getPositiveInteger(event.url.searchParams.get('exactPage'), 1);
	const likelyPage = getPositiveInteger(event.url.searchParams.get('likelyPage'), 1);
	const reasonParam = event.url.searchParams.get('reason') || '';
	const reason = ALLOWED_REASONS.has(reasonParam) ? reasonParam : '';

	const exactParams = new URLSearchParams({ page: String(exactPage), limit: String(limit) });
	if (reason) exactParams.set('reason', reason);

	const response = await api(`/archived-emails/duplicates/exact?${exactParams.toString()}`, event);
	const body = await response.json();
	if (!response.ok) {
		return error(response.status, body.message || 'Failed to load duplicate groups.');
	}

	const likelyResponse = await api(
		`/archived-emails/duplicates/likely?page=${likelyPage}&limit=${limit}`,
		event
	);
	const likelyBody = await likelyResponse.json();
	if (!likelyResponse.ok) {
		return error(
			likelyResponse.status,
			likelyBody.message || 'Failed to load likely duplicate groups.'
		);
	}

	return {
		duplicateGroups: body as ExactDuplicateGroupsResult,
		likelyDuplicateGroups: likelyBody as LikelyDuplicateGroupsResult,
		activeReason: reason,
	};
};
