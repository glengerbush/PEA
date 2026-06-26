import 'dotenv/config';

const isEnabled = (value: string | undefined) => ['true', '1'].includes(value?.toLowerCase() || '');
const isDisabled = (value: string | undefined) => value?.toLowerCase() === 'false';

export const app = {
	nodeEnv: process.env.NODE_ENV || 'development',
	port: process.env.PORT_BACKEND ? parseInt(process.env.PORT_BACKEND, 10) : 4000,
	encryptionKey: process.env.ENCRYPTION_KEY,
	syncFrequency: process.env.SYNC_FREQUENCY || '* * * * *', //default to 1 minute
	enableDeletion: process.env.ENABLE_DELETION === 'true',
	allInclusiveArchive: process.env.ALL_INCLUSIVE_ARCHIVE === 'true',
	isDemo: process.env.IS_DEMO === 'true',
	personalMode:
		!isDisabled(process.env.PERSONAL_MODE) && !isEnabled(process.env.VITE_ENTERPRISE_MODE),
};
