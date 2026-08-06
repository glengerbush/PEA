import { describe, expect, it } from 'vitest';
import { advanceSwipeMotion, createSwipeMotion, releaseSwipeMotion } from './swipe-back-motion';

describe('swipe-back motion', () => {
	it('tracks back-directed horizontal movement one-for-one', () => {
		let motion = createSwipeMotion();
		motion = advanceSwipeMotion(motion, {
			deltaX: -12,
			deltaY: 1,
			timeStamp: 0,
		});
		motion = advanceSwipeMotion(motion, {
			deltaX: -28,
			deltaY: 2,
			timeStamp: 16,
		});

		expect(motion.tracking).toBe(true);
		expect(motion.distance).toBe(40);
	});

	it('ignores the forward direction and vertically-dominant scrolling', () => {
		let motion = createSwipeMotion();
		motion = advanceSwipeMotion(motion, {
			deltaX: 30,
			deltaY: 0,
			timeStamp: 0,
		});
		motion = advanceSwipeMotion(motion, {
			deltaX: -12,
			deltaY: 20,
			timeStamp: 16,
		});

		expect(motion.tracking).toBe(false);
		expect(motion.distance).toBe(0);
	});

	it('commits after a short deliberate pull', () => {
		let motion = createSwipeMotion();
		for (let index = 0; index < 5; index += 1) {
			motion = advanceSwipeMotion(motion, {
				deltaX: -30,
				deltaY: 1,
				timeStamp: index * 80,
			});
		}

		expect(releaseSwipeMotion(motion, 1360, 410)).toBe('complete');
	});

	it('commits a fast flick but cancels the same short distance when slow', () => {
		let fast = createSwipeMotion();
		fast = advanceSwipeMotion(fast, { deltaX: -10, deltaY: 0, timeStamp: 0 });
		fast = advanceSwipeMotion(fast, { deltaX: -50, deltaY: 0, timeStamp: 40 });

		let slow = createSwipeMotion();
		slow = advanceSwipeMotion(slow, { deltaX: -10, deltaY: 0, timeStamp: 0 });
		slow = advanceSwipeMotion(slow, { deltaX: -50, deltaY: 0, timeStamp: 400 });

		expect(releaseSwipeMotion(fast, 1360, 120)).toBe('complete');
		expect(releaseSwipeMotion(slow, 1360, 480)).toBe('cancel');
	});
});
