import { loadTranslations } from '$lib/translations';
import type { LayoutLoad } from './$types';
import type { SupportedLanguage, SystemSettings, User } from '@pea/types';
import { api } from '$lib/api.load';
import { version } from '../../../../package.json';

// Static SPA: everything renders client-side against the local engine.
export const ssr = false;
export const prerender = false;

// Single-user desktop app: every request is the local user. The backend
// resolves the real identity independently; this constant only feeds the UI.
const LOCAL_USER = {
	id: 'local',
	email: 'local@localhost',
	first_name: 'Local',
	last_name: 'User',
	createdAt: new Date(),
} as Omit<User, 'passwordHash'>;

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

	const initLocale: SupportedLanguage = systemSettings?.language || 'en';
	await loadTranslations(initLocale, event.url.pathname);

	return {
		user: LOCAL_USER,
		enterpriseMode: false,
		personalMode: true,
		systemSettings,
		currentVersion: version,
		newVersionInfo: null,
	};
};
