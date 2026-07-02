import type { QueueJob as Job } from '../queue';
import { RemoteContentService } from '../../services/RemoteContentService';
import { logger } from '../../config/logger';

export default async function (job: Job<{ emailIds: string[] }>) {
	const { emailIds } = job.data;
	logger.info({ emailCount: emailIds.length }, 'Archiving remote email content');
	return RemoteContentService.archiveEmailRemoteContentBatch(emailIds);
}
