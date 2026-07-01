import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';

export const load: PageLoad = async () => {
	// LOCAL MODE: auth disabled and the Mailbox is the home page.
	throw redirect(307, '/mailbox');
};
