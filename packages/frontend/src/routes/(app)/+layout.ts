import { api } from '$lib/api.load';
import type { ContactMap } from '@pea/types';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async (event) => {
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

	// `user` flows down from the root layout.
	return {
		contacts,
	};
};
