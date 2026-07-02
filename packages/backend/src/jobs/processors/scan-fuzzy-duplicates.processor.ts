import type { QueueJob as Job } from '../queue';
import { DuplicateReviewService } from '../../services/DuplicateReviewService';
import { logger } from '../../config/logger';

export default async function (job: Job<{ batchSize?: number }>) {
	const { batchSize } = job.data;
	logger.info({ batchSize }, 'Scanning fuzzy duplicate candidates');
	return DuplicateReviewService.scanFuzzyDuplicateBatch(batchSize);
}
