import { and, asc, eq, inArray, lte, sql } from 'drizzle-orm';
import { Cron } from 'croner';
import { db } from '../database';
import { jobs } from '../database/schema';
import { config } from '../config';
import { logger } from '../config/logger';

/**
 * In-process job queue over the app's SQLite database (successor to
 * BullMQ→pg-boss). Single process + synchronous better-sqlite3 means claiming
 * is inherently race-free; crash recovery is a boot-time reset of 'active'
 * rows. Processors and their dispatch-by-name contract are unchanged.
 */

export type QueueName = 'ingestion' | 'indexing' | 'remote-content';
export const QUEUE_NAMES: QueueName[] = ['ingestion', 'indexing', 'remote-content'];

/** What processors receive — the (BullMQ-era) Job surface they actually use. */
export interface QueueJob<T = any> {
	id: string;
	name: string;
	data: T;
}

export interface SendOptions {
	/** Retries after the first attempt. 0 = run exactly once. Default 4 (= 5 tries). */
	retryLimit?: number;
	/** Suppress this send while a job with the same key is pending or running. */
	singletonKey?: string;
	/** Do not run before this time. */
	startAfter?: Date;
}

// Master/dispatcher jobs (initial-import, continuous-sync) create a sync session
// and fan out per-mailbox jobs; they are NOT idempotent, so a retry would create
// a duplicate session and re-dispatch every mailbox. Run them exactly once —
// unlike the idempotent batch jobs (index-email-batch, archive-remote-content-batch)
// which keep the default retry policy for transient failures.
export const masterJobOptions: SendOptions = { retryLimit: 0 };

type Dispatcher = (job: QueueJob) => Promise<unknown>;

interface QueueRuntime {
	dispatcher: Dispatcher;
	concurrency: number;
	running: number;
}

const runtimes = new Map<QueueName, QueueRuntime>();
const inFlight = new Set<Promise<void>>();
let poller: NodeJS.Timeout | null = null;
let retentionSweeper: NodeJS.Timeout | null = null;
let cron: Cron | null = null;
let stopping = false;

const POLL_MS = 500;
const BACKOFF_BASE_MS = 1000;
const BACKOFF_CAP_MS = 5 * 60 * 1000;

/** Registers the consumer for a queue (called by workers/index.ts at boot). */
export const registerWorker = (
	queue: QueueName,
	dispatcher: Dispatcher,
	concurrency: number
): void => {
	runtimes.set(queue, { dispatcher, concurrency, running: 0 });
};

const claimAndRun = (queue: QueueName, runtime: QueueRuntime): void => {
	const free = runtime.concurrency - runtime.running;
	if (free <= 0) {
		return;
	}
	const now = new Date();
	const claimable = db
		.select()
		.from(jobs)
		.where(and(eq(jobs.queue, queue), eq(jobs.state, 'pending'), lte(jobs.runAt, now)))
		.orderBy(asc(jobs.createdAt))
		.limit(free)
		.all();
	for (const job of claimable) {
		db.update(jobs)
			.set({ state: 'active', startedAt: now, attempts: job.attempts + 1 })
			.where(eq(jobs.id, job.id))
			.run();
		runtime.running++;
		const run = runtime
			.dispatcher({ id: job.id, name: job.name, data: job.payload })
			.then(() => {
				db.update(jobs)
					.set({ state: 'completed', finishedAt: new Date(), error: null })
					.where(eq(jobs.id, job.id))
					.run();
			})
			.catch((error: unknown) => {
				const attempts = job.attempts + 1;
				const message =
					error instanceof Error ? (error.stack ?? error.message) : String(error);
				if (attempts >= job.maxAttempts) {
					db.update(jobs)
						.set({ state: 'failed', finishedAt: new Date(), error: message })
						.where(eq(jobs.id, job.id))
						.run();
					logger.error({ queue, name: job.name, jobId: job.id, error: message }, 'Job failed permanently');
				} else {
					const delay = Math.min(BACKOFF_BASE_MS * 2 ** (attempts - 1), BACKOFF_CAP_MS);
					db.update(jobs)
						.set({ state: 'pending', runAt: new Date(Date.now() + delay), error: message })
						.where(eq(jobs.id, job.id))
						.run();
					logger.warn({ queue, name: job.name, jobId: job.id, attempts, delay }, 'Job failed, retrying');
				}
			})
			.finally(() => {
				runtime.running--;
				inFlight.delete(run);
			});
		inFlight.add(run);
	}
};

const tick = (): void => {
	if (stopping) {
		return;
	}
	for (const [queue, runtime] of runtimes) {
		try {
			claimAndRun(queue, runtime);
		} catch (error) {
			logger.error({ queue, error }, 'Queue tick failed');
		}
	}
};

/** Boot-time start: recover crash leftovers and begin polling. */
export const startQueue = async (): Promise<void> => {
	stopping = false;
	// Anything 'active' at boot is a leftover from a crash/kill — re-run it.
	const recovered = db
		.update(jobs)
		.set({ state: 'pending', runAt: new Date() })
		.where(eq(jobs.state, 'active'))
		.run();
	if (recovered.changes > 0) {
		logger.warn({ count: recovered.changes }, 'Recovered interrupted jobs from previous run');
	}
	poller = setInterval(tick, POLL_MS);
	// Retention: completed jobs age out after 2 days, failed after 14.
	retentionSweeper = setInterval(
		() => {
			try {
				db.run(sql`
					DELETE FROM jobs
					WHERE (state = 'completed' AND finished_at < (unixepoch() * 1000) - ${2 * 24 * 3600 * 1000})
						OR (state = 'failed' AND finished_at < (unixepoch() * 1000) - ${14 * 24 * 3600 * 1000})
				`);
			} catch (error) {
				logger.error({ error }, 'Job retention sweep failed');
			}
		},
		10 * 60 * 1000
	);
	retentionSweeper.unref();
	logger.info('Job queue started (SQLite, in-process)');
};

/** Graceful stop: cease claiming, wait for in-flight jobs (bounded by caller). */
export const stopQueue = async (): Promise<void> => {
	stopping = true;
	cron?.stop();
	cron = null;
	if (poller) {
		clearInterval(poller);
		poller = null;
	}
	if (retentionSweeper) {
		clearInterval(retentionSweeper);
		retentionSweeper = null;
	}
	await Promise.allSettled([...inFlight]);
};

/**
 * Enqueues a named job. Returns the job id, or null when suppressed by
 * options.singletonKey (a job with the same key is already pending/active).
 */
export const sendJob = async (
	queue: QueueName,
	name: string,
	payload: object,
	options: SendOptions = {}
): Promise<string | null> => {
	if (options.singletonKey) {
		const existing = db
			.select({ id: jobs.id })
			.from(jobs)
			.where(
				and(
					eq(jobs.queue, queue),
					eq(jobs.singletonKey, options.singletonKey),
					inArray(jobs.state, ['pending', 'active'])
				)
			)
			.limit(1)
			.all();
		if (existing.length > 0) {
			logger.info(
				{ queue, name, singletonKey: options.singletonKey },
				'Duplicate job suppressed (already queued or running).'
			);
			return null;
		}
	}
	const [row] = db
		.insert(jobs)
		.values({
			queue,
			name,
			payload,
			maxAttempts: (options.retryLimit ?? 4) + 1,
			runAt: options.startAfter ?? new Date(),
			singletonKey: options.singletonKey ?? null,
		})
		.returning({ id: jobs.id })
		.all();
	// Nudge the poller so locally-enqueued work starts without the poll delay.
	setImmediate(tick);
	return row?.id ?? null;
};

/**
 * Registers the continuous-sync cron (in-process via croner; pattern comes from
 * SYNC_FREQUENCY). The singletonKey makes a tick that fires while the previous
 * one is still queued a no-op.
 */
export const registerSyncSchedule = async (): Promise<void> => {
	cron?.stop();
	cron = new Cron(config.app.syncFrequency, () => {
		void sendJob(
			'ingestion',
			'schedule-continuous-sync',
			{},
			{ retryLimit: 0, singletonKey: 'schedule-continuous-sync' }
		);
	});
	logger.info({ pattern: config.app.syncFrequency }, 'Continuous sync schedule registered');
};

/**
 * Removes queued/retryable/failed ingestion jobs belonging to a source (force
 * sync cleanup). Active jobs are left alone — same net behavior as before.
 */
export const removeJobsBySourceId = async (ingestionSourceId: string): Promise<number> => {
	const result = db.run(sql`
		DELETE FROM jobs
		WHERE queue = 'ingestion'
			AND state IN ('pending', 'failed')
			AND json_extract(payload, '$.ingestionSourceId') = ${ingestionSourceId}
	`);
	const count = Number(result.changes ?? 0);
	logger.info({ ingestionSourceId, count }, 'Removed queued jobs for source during force sync.');
	return count;
};

/** Builds a queue dispatcher from a named-job processor map. */
export const makeDispatcher = (
	processors: Record<string, (job: QueueJob) => Promise<unknown>>
): Dispatcher => {
	return async (job: QueueJob): Promise<void> => {
		const processor = processors[job.name];
		if (!processor) {
			throw new Error(`Unknown job name: ${job.name}`);
		}
		await processor(job);
	};
};
