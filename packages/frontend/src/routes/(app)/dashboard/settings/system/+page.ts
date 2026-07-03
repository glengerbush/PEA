import { api } from '$lib/api.load';
import type { SystemSettings } from '@pea/types';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';

export const load: PageLoad = async (event) => {
	const response = await api('/settings/system', event);

	if (!response.ok) {
		const { message } = await response.json();
		throw error(response.status, message || 'Failed to fetch system settings');
	}

	const systemSettings: SystemSettings = await response.json();
	return {
		systemSettings,
	};
};
