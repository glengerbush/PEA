import type { Handle } from '@sveltejs/kit';
import type { User } from '@open-archiver/types';
import 'dotenv/config';

const isEnabled = (value: unknown) =>
	value === true || (typeof value === 'string' && ['true', '1'].includes(value.toLowerCase()));
const isDisabled = (value: unknown) => typeof value === 'string' && value.toLowerCase() === 'false';

// LOCAL MODE: authentication is disabled. Every request is treated as the local
// user so route guards pass and no login/setup screens are shown. The backend
// resolves the real user identity independently for all API calls, so this
// placeholder is only used for the SvelteKit guards. Restore JWT verification
// here to re-enable auth.
const LOCAL_USER = {
	id: 'local',
	email: 'local@localhost',
	first_name: 'Local',
	last_name: 'User',
	createdAt: new Date(),
} as Omit<User, 'passwordHash'>;

export const handle: Handle = async ({ event, resolve }) => {
	event.locals.user = LOCAL_USER;
	event.locals.accessToken = null;

	const enterpriseMode =
		isEnabled(import.meta.env.VITE_ENTERPRISE_MODE) ||
		isEnabled(process.env.VITE_ENTERPRISE_MODE);
	event.locals.enterpriseMode = enterpriseMode;
	event.locals.personalMode = !enterpriseMode && !isDisabled(process.env.PERSONAL_MODE);

	return resolve(event);
};
