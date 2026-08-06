import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	SNAPSHOT_SETTLE_DELAY_MS,
	SwipeBackSnapshotScheduler,
} from './swipe-back-snapshot-scheduler';

describe('swipe-back snapshot scheduling', () => {
	afterEach(() => vi.useRealTimers());

	it('captures shortly after navigation settles', () => {
		vi.useFakeTimers();
		const scheduler = new SwipeBackSnapshotScheduler();
		const capture = vi.fn();

		scheduler.schedule(capture);
		vi.advanceTimersByTime(SNAPSHOT_SETTLE_DELAY_MS - 1);
		expect(capture).not.toHaveBeenCalled();

		vi.advanceTimersByTime(1);
		expect(capture).toHaveBeenCalledOnce();
	});

	it('coalesces repeated capture requests', () => {
		vi.useFakeTimers();
		const scheduler = new SwipeBackSnapshotScheduler();
		const capture = vi.fn();

		scheduler.schedule(capture);
		vi.advanceTimersByTime(SNAPSHOT_SETTLE_DELAY_MS - 1);
		scheduler.schedule(capture);
		vi.advanceTimersByTime(SNAPSHOT_SETTLE_DELAY_MS);

		expect(capture).toHaveBeenCalledOnce();
	});
});
