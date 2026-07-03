import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ url }) => {
	const params = new URLSearchParams();
	const query = url.searchParams.get('q') || url.searchParams.get('keywords') || '';

	if (query) params.set('q', query);

	for (const key of [
		'fields',
		'page',
		'limit',
		'matchingStrategy',
		'ingestionSourceId',
		'hasAttachments',
		'sourcePath',
		'tags',
		'sort',
		'direction',
	] as const) {
		const value = url.searchParams.get(key);
		if (value) params.set(key, value);
	}

	const suffix = params.toString();
	throw redirect(307, `/mailbox${suffix ? `?${suffix}` : ''}`);
};
