import { api } from '$lib/server/api';
import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import type { ArchivedEmail } from '@open-archiver/types';

export const load: PageServerLoad = async (event) => {
	try {
		const { id } = event.params;

		const emailResponse = await api(`/archived-emails/${id}`, event);

		if (!emailResponse.ok) {
			const responseText = await emailResponse.json();
			return error(emailResponse.status, responseText.message || 'Unable to read this email.');
		}

		const email: ArchivedEmail = await emailResponse.json();

		// Existing tags across the archive, used to power the tag combobox.
		let allTags: string[] = [];
		const facetsResponse = await api('/archived-emails/facets', event);
		if (facetsResponse.ok) {
			const facets = (await facetsResponse.json()) as { tags?: string[] };
			allTags = Array.isArray(facets.tags) ? facets.tags : [];
		}

		return { email, allTags };
	} catch (e) {
		console.error('Failed to load archived email:', e);
		return {
			email: null,
			allTags: [],
			error: 'Failed to load email',
		};
	}
};
