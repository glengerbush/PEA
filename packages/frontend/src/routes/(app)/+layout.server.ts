import { redirect } from '@sveltejs/kit';
import { api } from '$lib/server/api';
import type { ContactMap } from '@open-archiver/types';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = async (event) => {
	if (!event.locals.user) {
		throw redirect(302, '/signin');
	}

	// Imported contacts (email → display name), used to show names next to addresses.
	let contacts: ContactMap = {};
	try {
		const res = await api('/contacts/map', event);
		if (res.ok) {
			contacts = (await res.json()) as ContactMap;
		}
	} catch {
		// Non-fatal — names just won't resolve.
	}

	return {
		user: event.locals.user,
		contacts,
	};
};
