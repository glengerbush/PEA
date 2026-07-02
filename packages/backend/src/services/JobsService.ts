import { and, asc, eq, gt, lte, sql } from 'drizzle-orm';
import { db } from '../database';
import { jobs } from '../database/schema';
import { QUEUE_NAMES } from '../jobs/queue';
import { IJob, IQueueCounts, IQueueDetails, IQueueOverview, JobStatus } from '@open-archiver/types';

/**
 * Jobs admin introspection over the in-app queue table. Maps queue states onto
 * the UI's BullMQ-era status vocabulary: waiting ← pending (due), delayed ←
 * pending (scheduled ahead), active/completed/failed 1:1, paused unused.
 */

const emptyCounts = (): IQueueCounts => ({
	active: 0,
	completed: 0,
	failed: 0,
	delayed: 0,
	waiting: 0,
	paused: 0,
});

const statusPredicate = (status: JobStatus) => {
	switch (status) {
		case 'active':
			return eq(jobs.state, 'active');
		case 'completed':
			return eq(jobs.state, 'completed');
		case 'failed':
			return eq(jobs.state, 'failed');
		case 'waiting':
			return and(eq(jobs.state, 'pending'), lte(jobs.runAt, new Date()));
		case 'delayed':
			return and(eq(jobs.state, 'pending'), gt(jobs.runAt, new Date()));
		case 'paused':
			return sql`0 = 1`;
		default:
			throw new Error(`Unknown job status ${status}`);
	}
};

export class JobsService {
	public async getQueues(): Promise<IQueueOverview[]> {
		const rows = db
			.select({
				queue: jobs.queue,
				active: sql<number>`count(*) FILTER (WHERE state = 'active')`,
				completed: sql<number>`count(*) FILTER (WHERE state = 'completed')`,
				failed: sql<number>`count(*) FILTER (WHERE state = 'failed')`,
				delayed: sql<number>`count(*) FILTER (WHERE state = 'pending' AND run_at > (unixepoch() * 1000))`,
				waiting: sql<number>`count(*) FILTER (WHERE state = 'pending' AND run_at <= (unixepoch() * 1000))`,
			})
			.from(jobs)
			.groupBy(jobs.queue)
			.all();
		const byName = new Map(rows.map((row) => [row.queue, row]));
		return QUEUE_NAMES.map((name) => {
			const row = byName.get(name);
			return {
				name,
				counts: row
					? {
							active: row.active,
							completed: row.completed,
							failed: row.failed,
							delayed: row.delayed,
							waiting: row.waiting,
							paused: 0,
						}
					: emptyCounts(),
			};
		});
	}

	public async getQueueDetails(
		queueName: string,
		status: JobStatus,
		page: number,
		limit: number
	): Promise<IQueueDetails> {
		if (!(QUEUE_NAMES as string[]).includes(queueName)) {
			throw new Error(`Queue ${queueName} not found`);
		}
		const predicate = and(eq(jobs.queue, queueName), statusPredicate(status));

		const overview = await this.getQueues();
		const counts = overview.find((queue) => queue.name === queueName)!.counts;

		const rows = db
			.select()
			.from(jobs)
			.where(predicate)
			.orderBy(asc(jobs.createdAt))
			.limit(limit)
			.offset((page - 1) * limit)
			.all();
		const [{ count: totalJobs }] = db
			.select({ count: sql<number>`count(*)` })
			.from(jobs)
			.where(predicate)
			.all();

		return {
			name: queueName,
			counts,
			jobs: rows.map((row) => this.formatJob(row, status)),
			pagination: {
				currentPage: page,
				totalPages: Math.ceil(totalJobs / limit),
				totalJobs,
				limit,
			},
		};
	}

	private formatJob(row: typeof jobs.$inferSelect, status: JobStatus): IJob {
		const payload = (row.payload ?? {}) as Record<string, unknown>;
		return {
			id: row.id,
			name: row.name,
			data: payload,
			state: status,
			failedReason: row.error?.split('\n')[0],
			timestamp: row.createdAt?.getTime() ?? 0,
			processedOn: row.startedAt?.getTime(),
			finishedOn: row.finishedAt?.getTime(),
			attemptsMade: row.attempts,
			stacktrace: row.error ? [row.error] : [],
			returnValue: null,
			ingestionSourceId: payload.ingestionSourceId as string | undefined,
			error: status === 'failed' ? row.error : undefined,
		};
	}
}
