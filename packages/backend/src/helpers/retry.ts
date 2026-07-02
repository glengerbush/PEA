import { logger } from '../config/logger';

interface RetryOptions {
	label: string;
	retries?: number;
	delayMs?: number;
	/** Return false to abort retrying and rethrow immediately (e.g. non-transient errors). */
	shouldRetry?: (error: unknown) => boolean;
}

/**
 * Retries an async operation with a fixed delay. Used at startup to gate on
 * backing services (database, Meilisearch) coming up, so a single-process boot
 * doesn't crash when it races service startup.
 */
export async function withRetry<T>(fn: () => Promise<T>, options: RetryOptions): Promise<T> {
	const retries = options.retries ?? 30;
	const delayMs = options.delayMs ?? 2000;
	let lastError: unknown;
	for (let attempt = 1; attempt <= retries; attempt++) {
		try {
			return await fn();
		} catch (error) {
			lastError = error;
			if (options.shouldRetry && !options.shouldRetry(error)) {
				throw error;
			}
			if (attempt < retries) {
				logger.warn(
					{ attempt, retries, error: error instanceof Error ? error.message : error },
					`${options.label} failed, retrying in ${delayMs}ms`
				);
				await new Promise((resolve) => setTimeout(resolve, delayMs));
			}
		}
	}
	throw lastError;
}
