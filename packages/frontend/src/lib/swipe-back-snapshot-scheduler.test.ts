import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	SNAPSHOT_QUIET_PERIOD_MS,
	SwipeBackSnapshotScheduler,
} from './swipe-back-snapshot-scheduler';

describe('swipe-back snapshot scheduling', () => {
	afterEach(() => vi.useRealTimers());

	it('waits for a quiet period after pointer activity', () => {
		vi.useFakeTimers();
		const scheduler = new SwipeBackSnapshotScheduler();
		const capture = vi.fn();

		scheduler.schedule(capture);
		vi.advanceTimersByTime(SNAPSHOT_QUIET_PERIOD_MS - 100);
		scheduler.deferIfPending(capture);
		vi.advanceTimersByTime(SNAPSHOT_QUIET_PERIOD_MS - 1);
		expect(capture).not.toHaveBeenCalled();

		vi.advanceTimersByTime(1);
		expect(capture).toHaveBeenCalledOnce();
	});

	it('does not schedule new work for activity after capture', () => {
		vi.useFakeTimers();
		const scheduler = new SwipeBackSnapshotScheduler();
		const capture = vi.fn();

		scheduler.schedule(capture);
		vi.advanceTimersByTime(SNAPSHOT_QUIET_PERIOD_MS);
		scheduler.deferIfPending(capture);
		vi.runAllTimers();

		expect(capture).toHaveBeenCalledOnce();
	});
});
