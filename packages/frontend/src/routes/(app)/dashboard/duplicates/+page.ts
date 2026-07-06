import { api } from '$lib/api.load';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import type { ExactDuplicateGroupsResult } from '@pea/types';

function getPositiveInteger(value: string | null, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

const ALLOWED_REASONS = new Set([
	'message_id',
	'storage_hash',
	'attachment_hash_set',
	'sender_recipients_sent',
	'message_body',
]);

export const load: PageLoad = async (event) => {
	const limit = Math.min(getPositiveInteger(event.url.searchParams.get('limit'), 25), 100);
	const exactPage = getPositiveInteger(event.url.searchParams.get('exactPage'), 1);
	const reasonParam = event.url.searchParams.get('reason') || '';
	const reason = ALLOWED_REASONS.has(reasonParam) ? reasonParam : '';

	const exactParams = new URLSearchParams({ page: String(exactPage), limit: String(limit) });
	if (reason) exactParams.set('reason', reason);

	const response = await api(`/archived-emails/duplicates/exact?${exactParams.toString()}`, event);
	const body = await response.json();
	if (!response.ok) {
		return error(response.status, body.message || 'Failed to load duplicate groups.');
	}

	return {
		duplicateGroups: body as ExactDuplicateGroupsResult,
		activeReason: reason,
	};
};
