export interface SwipeMotionInput {
	deltaX: number;
	deltaY: number;
	timeStamp: number;
}

export interface SwipeMotion {
	distance: number;
	tracking: boolean;
	intentDistance: number;
	lastTimeStamp: number | null;
	flickTimeStamp: number | null;
}

export type SwipeRelease = 'complete' | 'cancel';

const START_SLOP = 8;
const HORIZONTAL_DOMINANCE = 1.2;
const FLICK_MIN_DISTANCE = 50;
const FLICK_VELOCITY = 0.65;
const FLICK_MAX_AGE_MS = 120;

export function createSwipeMotion(): SwipeMotion {
	return {
		distance: 0,
		tracking: false,
		intentDistance: 0,
		lastTimeStamp: null,
		flickTimeStamp: null,
	};
}

/**
 * Trackpad back is negative deltaX: the content follows a rightward finger
 * movement. Once locked, distance remains a direct pixel-for-pixel mapping.
 */
export function advanceSwipeMotion(motion: SwipeMotion, input: SwipeMotionInput): SwipeMotion {
	const backDelta = -input.deltaX;
	const horizontal = Math.abs(input.deltaX);
	const vertical = Math.abs(input.deltaY);

	if (!motion.tracking) {
		if (backDelta <= 0 || horizontal <= vertical * HORIZONTAL_DOMINANCE) {
			return { ...motion, intentDistance: 0, lastTimeStamp: input.timeStamp };
		}

		const intentDistance = motion.intentDistance + backDelta;
		if (intentDistance < START_SLOP) {
			return { ...motion, intentDistance, lastTimeStamp: input.timeStamp };
		}

		return {
			...motion,
			distance: intentDistance,
			tracking: true,
			intentDistance,
			lastTimeStamp: input.timeStamp,
		};
	}

	// Ignore vertical wheel noise after locking, while retaining ownership of
	// the gesture so native overscroll cannot start underneath the transition.
	if (horizontal < vertical * 0.45) {
		return { ...motion, lastTimeStamp: input.timeStamp };
	}

	const distance = Math.max(0, motion.distance + backDelta);
	const elapsed =
		motion.lastTimeStamp === null ? 0 : Math.max(0, input.timeStamp - motion.lastTimeStamp);
	const velocity = elapsed > 0 && backDelta > 0 ? backDelta / elapsed : 0;

	return {
		...motion,
		distance,
		intentDistance: distance,
		lastTimeStamp: input.timeStamp,
		flickTimeStamp: velocity >= FLICK_VELOCITY ? input.timeStamp : motion.flickTimeStamp,
	};
}

export function swipeCommitDistance(viewportWidth: number): number {
	return Math.min(150, Math.max(120, viewportWidth * 0.11));
}

export function releaseSwipeMotion(
	motion: SwipeMotion,
	viewportWidth: number,
	timeStamp: number
): SwipeRelease {
	if (motion.distance >= swipeCommitDistance(viewportWidth)) return 'complete';

	const recentFlick =
		motion.distance >= FLICK_MIN_DISTANCE &&
		motion.flickTimeStamp !== null &&
		timeStamp - motion.flickTimeStamp <= FLICK_MAX_AGE_MS;
	return recentFlick ? 'complete' : 'cancel';
}
