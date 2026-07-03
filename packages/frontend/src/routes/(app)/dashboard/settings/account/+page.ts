import type { PageLoad } from './$types';
import { api } from '$lib/api.load';
import type { User } from '@pea/types';

export const load: PageLoad = async (event) => {
	const response = await api('/users/profile', event);
	if (!response.ok) {
		const error = await response.json();
		console.error('Failed to fetch profile:', error);
		// Return null user if failed, handle in UI
		return { user: null };
	}
	const user: User = await response.json();
	return { user };
};
