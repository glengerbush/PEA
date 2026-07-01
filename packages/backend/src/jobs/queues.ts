import { Queue } from 'bullmq';
import { connection } from '../config/redis';

// Default job options
const defaultJobOptions = {
	attempts: 5,
	backoff: {
		type: 'exponential',
		delay: 1000,
	},
	removeOnComplete: {
		count: 1000,
	},
	removeOnFail: {
		count: 5000,
	},
};

// Master/dispatcher jobs (initial-import, continuous-sync) create a sync session
// and fan out per-mailbox jobs; they are NOT idempotent, so a retry would create
// a duplicate session and re-dispatch every mailbox. Run them exactly once —
// unlike the idempotent batch jobs (index-email-batch, archive-remote-content-batch)
// which keep the default retry policy for transient failures.
export const masterJobOptions = { attempts: 1 };

export const ingestionQueue = new Queue('ingestion', {
	connection,
	defaultJobOptions,
});

export const indexingQueue = new Queue('indexing', {
	connection,
	defaultJobOptions,
});

export const remoteContentQueue = new Queue('remote-content', {
	connection,
	defaultJobOptions,
});
