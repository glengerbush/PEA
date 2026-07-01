import { api } from '$lib/server/api';
import { error } from '@sveltejs/kit';
import type { RemoteContentIssuesResult } from '@open-archiver/types';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async (event) => {
	const params = event.url.searchParams;
	const page = Math.max(1, parseInt(params.get('page') ?? '1', 10) || 1);
	const limit = params.get('limit') ?? '25';
	const status = params.get('status') ?? 'all';
	const sort = params.get('sort') ?? 'date';
	const direction = params.get('direction') ?? 'desc';

	const qs = new URLSearchParams({ page: String(page), limit, status, sort, direction });
	const response = await api(`/dashboard/remote-content-issues?${qs.toString()}`, event);
	if (!response.ok) {
		const body = await response.json().catch(() => ({}));
		throw error(response.status, body.message || 'Failed to load remote content issues');
	}
	const result: RemoteContentIssuesResult = await response.json();

	return {
		result,
		filters: { status, sort, direction, limit },
	};
};
