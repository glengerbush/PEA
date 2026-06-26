import { Worker } from 'bullmq';
import { connection } from '../config/redis';
import archiveRemoteContentBatchProcessor from '../jobs/processors/archive-remote-content-batch.processor';
import { logger } from '../config/logger';

const processor = async (job: any) => {
	switch (job.name) {
		case 'archive-remote-content-batch':
			return archiveRemoteContentBatchProcessor(job);
		default:
			throw new Error(`Unknown job name: ${job.name}`);
	}
};

const worker = new Worker('remote-content', processor, {
	connection,
	concurrency: 2,
	removeOnComplete: {
		count: 100,
	},
	removeOnFail: {
		count: 500,
	},
});

logger.info('Remote content worker started');

process.on('SIGINT', () => worker.close());
process.on('SIGTERM', () => worker.close());
