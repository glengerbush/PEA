import { registerWorker, makeDispatcher } from '../jobs/queue';
import initialImportProcessor from '../jobs/processors/initial-import.processor';
import continuousSyncProcessor from '../jobs/processors/continuous-sync.processor';
import scheduleContinuousSyncProcessor from '../jobs/processors/schedule-continuous-sync.processor';
import { processMailboxProcessor } from '../jobs/processors/process-mailbox.processor';
import syncCycleFinishedProcessor from '../jobs/processors/sync-cycle-finished.processor';
import indexEmailBatchProcessor from '../jobs/processors/index-email-batch.processor';
import scanFuzzyDuplicatesProcessor from '../jobs/processors/scan-fuzzy-duplicates.processor';
import archiveRemoteContentBatchProcessor from '../jobs/processors/archive-remote-content-batch.processor';
import { logger } from '../config/logger';

// All three queue consumers run in-process. The processors themselves are
// unchanged; only the queue engine has moved (pg-boss → in-app SQLite queue).

const ingestionDispatcher = makeDispatcher({
	'initial-import': initialImportProcessor,
	'sync-cycle-finished': syncCycleFinishedProcessor,
	'continuous-sync': continuousSyncProcessor,
	'schedule-continuous-sync': scheduleContinuousSyncProcessor,
	'process-mailbox': processMailboxProcessor,
});

const indexingDispatcher = makeDispatcher({
	'index-email-batch': indexEmailBatchProcessor,
	'scan-fuzzy-duplicates': scanFuzzyDuplicatesProcessor,
});

const remoteContentDispatcher = makeDispatcher({
	'archive-remote-content-batch': archiveRemoteContentBatchProcessor,
});

/** Registers the queue consumers (concurrency mirrors the former worker config). */
export const startWorkers = async (): Promise<void> => {
	registerWorker(
		'ingestion',
		ingestionDispatcher,
		// Configurable via INGESTION_WORKER_CONCURRENCY env var. Tune based on available RAM.
		process.env.INGESTION_WORKER_CONCURRENCY
			? parseInt(process.env.INGESTION_WORKER_CONCURRENCY, 10)
			: 5
	);
	registerWorker('indexing', indexingDispatcher, 1);
	registerWorker('remote-content', remoteContentDispatcher, 2);
	logger.info('In-process job workers registered (ingestion, indexing, remote-content)');
};
