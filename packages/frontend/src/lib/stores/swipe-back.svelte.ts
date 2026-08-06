import {
	advanceSwipeMotion,
	createSwipeMotion,
	releaseSwipeMotion,
	type SwipeMotion,
} from '$lib/swipe-back-motion';
import { getSwipeBackSnapshot, type SwipeBackSnapshot } from '$lib/swipe-back-snapshot';

export interface SwipeWheelInput {
	deltaX: number;
	deltaY: number;
	deltaMode?: number;
	timeStamp?: number;
	preventDefault?: () => void;
}

export interface SwipeBackOptions {
	onComplete: () => Promise<void>;
	targetUrl: () => string;
	isDisabled?: () => boolean;
}

export type SwipeBackPhase = 'idle' | 'tracking' | 'completing' | 'cancelling';

const RELEASE_DELAY_MS = 90;
const COMPLETE_DURATION_MS = 210;
const CANCEL_DURATION_MS = 150;

/** One app-wide gesture coordinator, rendered by the root layout. */
class SwipeBackGesture {
	distance = $state(0);
	phase = $state<SwipeBackPhase>('idle');
	viewportWidth = $state(1);
	snapshot = $state.raw<SwipeBackSnapshot | null>(null);
	reducedMotion = $state(false);

	#motion: SwipeMotion = createSwipeMotion();
	#options: SwipeBackOptions | null = null;
	#configuration: symbol | null = null;
	#releaseTimer: ReturnType<typeof setTimeout> | null = null;
	#settleTimer: ReturnType<typeof setTimeout> | null = null;

	get active(): boolean {
		return this.phase !== 'idle';
	}

	get progress(): number {
		return Math.min(1, Math.max(0, this.distance / this.viewportWidth));
	}

	get visualDistance(): number {
		return this.reducedMotion ? 0 : this.distance;
	}

	get transitionDurationMs(): number {
		if (this.reducedMotion) return 0;
		if (this.phase === 'completing') return COMPLETE_DURATION_MS;
		if (this.phase === 'cancelling') return CANCEL_DURATION_MS;
		return 0;
	}

	setReducedMotion(reduced: boolean): void {
		this.reducedMotion = reduced;
	}

	configure(options: SwipeBackOptions): () => void {
		const configuration = Symbol('swipe-back-configuration');
		this.#configuration = configuration;
		this.#options = options;

		return () => {
			if (this.#configuration !== configuration) return;
			this.#configuration = null;
			this.#options = null;
			// Completion owns cleanup while navigation swaps the route underneath.
			if (this.phase !== 'completing') this.#reset();
		};
	}

	handleWheel = (event: SwipeWheelInput): void => {
		const options = this.#options;
		if (!options || options.isDisabled?.()) {
			if (this.active) this.#cancel();
			return;
		}
		if (this.phase === 'completing' || this.phase === 'cancelling') return;

		const timeStamp = event.timeStamp ?? performance.now();
		const next = advanceSwipeMotion(this.#motion, {
			deltaX: this.#normalizeDelta(event.deltaX, event.deltaMode),
			deltaY: this.#normalizeDelta(event.deltaY, event.deltaMode),
			timeStamp,
		});

		if (!next.tracking) {
			this.#motion = next;
			return;
		}

		event.preventDefault?.();
		if (!this.#motion.tracking) {
			this.viewportWidth = Math.max(1, window.innerWidth);
			this.snapshot = getSwipeBackSnapshot(options.targetUrl());
			this.phase = 'tracking';
		}

		this.#motion = next;
		this.distance = Math.min(this.viewportWidth, next.distance);
		this.#scheduleRelease(timeStamp);

		if (this.distance >= this.viewportWidth) this.#settle(timeStamp, 'complete');
	};

	cancel(): void {
		if (this.active) this.#cancel();
	}

	#scheduleRelease(lastEventTime: number): void {
		if (this.#releaseTimer) clearTimeout(this.#releaseTimer);
		this.#releaseTimer = setTimeout(() => {
			this.#releaseTimer = null;
			this.#settle(lastEventTime + RELEASE_DELAY_MS);
		}, RELEASE_DELAY_MS);
	}

	#settle(timeStamp: number, forced?: 'complete'): void {
		if (this.phase !== 'tracking') return;
		const decision = forced ?? releaseSwipeMotion(this.#motion, this.viewportWidth, timeStamp);
		if (decision === 'cancel') {
			this.#cancel();
			return;
		}

		if (this.#releaseTimer) clearTimeout(this.#releaseTimer);
		this.#releaseTimer = null;
		const complete = this.#options?.onComplete;
		this.phase = 'completing';
		this.distance = this.viewportWidth;
		this.#settleTimer = setTimeout(
			async () => {
				this.#settleTimer = null;
				try {
					await complete?.();
				} finally {
					this.#reset();
				}
			},
			this.reducedMotion ? 0 : COMPLETE_DURATION_MS
		);
	}

	#cancel(): void {
		if (this.#releaseTimer) clearTimeout(this.#releaseTimer);
		this.#releaseTimer = null;
		this.phase = 'cancelling';
		this.distance = 0;
		this.#settleTimer = setTimeout(
			() => {
				this.#settleTimer = null;
				this.#reset();
			},
			this.reducedMotion ? 0 : CANCEL_DURATION_MS
		);
	}

	#reset(): void {
		if (this.#releaseTimer) clearTimeout(this.#releaseTimer);
		if (this.#settleTimer) clearTimeout(this.#settleTimer);
		this.#releaseTimer = null;
		this.#settleTimer = null;
		this.#motion = createSwipeMotion();
		this.distance = 0;
		this.phase = 'idle';
		this.snapshot = null;
	}

	#normalizeDelta(value: number, mode = 0): number {
		if (mode === 1) return value * 16;
		if (mode === 2) return value * window.innerWidth;
		return value;
	}
}

export const swipeBackGesture = new SwipeBackGesture();
