import 'dotenv/config';

export const app = {
	nodeEnv: process.env.NODE_ENV || 'development',
	port: process.env.PORT_BACKEND ? parseInt(process.env.PORT_BACKEND, 10) : 4000,
	encryptionKey: process.env.ENCRYPTION_KEY,
	syncFrequency: process.env.SYNC_FREQUENCY || '* * * * *', //default to 1 minute
	isDemo: process.env.IS_DEMO === 'true',
	/** Emails per index-email-batch job. */
	indexingBatchSize: process.env.OA_INDEXING_BATCH
		? parseInt(process.env.OA_INDEXING_BATCH, 10)
		: 500,
	// Legacy commit-based update check (the desktop app updates itself via the
	// Tauri updater; without OA_GIT_SHA this reports status 'unknown').
	gitSha: process.env.OA_GIT_SHA || 'unknown',
	updateRepo: process.env.OA_UPDATE_REPO || 'glengerbush/OpenArchiver',
	updateBranch: process.env.OA_UPDATE_BRANCH || 'main',
	updateCommand: process.env.OA_UPDATE_COMMAND || '',
};
