import type { LayoutLoad } from './$types';
import type { SystemSettings } from '@pea/types';
import { api } from '$lib/api.load';
import { version } from '../../../../package.json';

// Static SPA: everything renders client-side against the local engine.
export const ssr = false;
export const prerender = false;

export const load: LayoutLoad = async (event) => {
	let systemSettings: SystemSettings | null = null;
	try {
		const response = await api('/settings/system', event);
		if (response.ok) {
			systemSettings = await response.json();
		}
	} catch {
		// Engine still booting — defaults keep the shell rendering.
	}

	return {
		systemSettings,
		currentVersion: version,
	};
};
