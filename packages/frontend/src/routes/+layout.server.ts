import type { LayoutServerLoad } from './$types';
import 'dotenv/config';
import { api } from '$lib/server/api';
import type { SystemSettings } from '@open-archiver/types';
import { version } from '../../../../package.json';

export const load: LayoutServerLoad = async (event) => {
	const { locals } = event;
	// LOCAL MODE: auth is disabled, so the setup/signin redirect flow is skipped.

	const systemSettingsResponse = await api('/settings/system', event);
	const systemSettings: SystemSettings | null = systemSettingsResponse.ok
		? await systemSettingsResponse.json()
		: null;

	return {
		user: locals.user,
		accessToken: locals.accessToken,
		enterpriseMode: locals.enterpriseMode,
		personalMode: locals.personalMode,
		systemSettings,
		currentVersion: version,
		newVersionInfo: null,
	};
};
