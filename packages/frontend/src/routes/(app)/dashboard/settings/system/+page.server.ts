import { api } from '$lib/server/api';
import type { SystemSettings, UpdateCheckResult } from '@open-archiver/types';
import { error, fail } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async (event) => {
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

export const actions: Actions = {
	save: async (event) => {
		const formData = await event.request.formData();

		// Only send keys the form actually submitted — the backend merges the
		// partial body into the current settings. Building a "full" body would
		// overwrite fields this form has no inputs for (timeZone/clockFormat),
		// wiping them to null/'12h' on every save.
		const body: Partial<SystemSettings> = {};
		if (formData.has('language')) {
			body.language = formData.get('language') as SystemSettings['language'];
		}
		if (formData.has('theme')) {
			body.theme = formData.get('theme') as SystemSettings['theme'];
		}
		if (formData.has('timeZone')) {
			const timeZone = formData.get('timeZone');
			body.timeZone = timeZone ? String(timeZone) : null;
		}
		if (formData.has('clockFormat')) {
			body.clockFormat = formData.get('clockFormat') === '24h' ? '24h' : '12h';
		}

		const response = await api('/settings/system', event, {
			method: 'PUT',
			body: JSON.stringify(body),
		});

		if (!response.ok) {
			const { message } = await response.json();
			return fail(response.status, { message: message || 'Failed to update settings' });
		}

		const updatedSettings: SystemSettings = await response.json();

		return {
			success: true,
			settings: updatedSettings,
		};
	},

	checkUpdates: async (event) => {
		const response = await api('/settings/updates/check', event);

		if (!response.ok) {
			const { message } = await response
				.json()
				.catch(() => ({ message: 'Update check failed' }));
			return fail(response.status, { updateError: message || 'Update check failed' });
		}

		const update: UpdateCheckResult = await response.json();
		return { update };
	},
};
