import 'dotenv/config';

export const app = {
	nodeEnv: process.env.NODE_ENV || 'development',
	port: process.env.PORT_BACKEND ? parseInt(process.env.PORT_BACKEND, 10) : 4000,
	encryptionKey: process.env.ENCRYPTION_KEY,
	syncFrequency: process.env.SYNC_FREQUENCY || '* * * * *', //default to 1 minute
	isDemo: process.env.IS_DEMO === 'true',
	// Update-check: the commit stamped into the image at build time, and the
	// GitHub repo/branch to compare against. `updateCommand` is what the user
	// runs on the host to apply an update (see update-local.sh).
	gitSha: process.env.OA_GIT_SHA || 'unknown',
	updateRepo: process.env.OA_UPDATE_REPO || 'glengerbush/OpenArchiver',
	updateBranch: process.env.OA_UPDATE_BRANCH || 'main',
	updateCommand: process.env.OA_UPDATE_COMMAND || './update-local.sh',
};
