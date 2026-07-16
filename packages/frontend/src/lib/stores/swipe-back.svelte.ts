export interface SwipeWheelInput {
	deltaX: number;
	deltaY: number;
	deltaMode?: number;
	preventDefault?: () => void;
}

interface SwipeBackOptions {
	onComplete: () => void;
	isDisabled?: () => boolean;
}

/**
 * Turns trackpad wheel deltas into a deliberate, animated back gesture.
 *
 * The gesture locks onto the first clearly-horizontal movement, tolerates the
 * small vertical wobble common to trackpads, and eases both toward and away
 * from the edge instead of flashing at a hard threshold.
 */
export class SwipeBackGesture {
	progress = $state(0);

	#distance = 0;
	#direction = 0;
	#targetProgress = 0;
	#frame: number | null = null;
	#releaseTimer: ReturnType<typeof setTimeout> | null = null;
	#completeTimer: ReturnType<typeof setTimeout> | null = null;
	#completing = false;
	#onComplete: () => void;
	#isDisabled: () => boolean;

	constructor({ onComplete, isDisabled = () => false }: SwipeBackOptions) {
		this.#onComplete = onComplete;
		this.#isDisabled = isDisabled;
	}

	handleWheel = (event: SwipeWheelInput): void => {
		if (this.#isDisabled()) {
			this.#release();
			return;
		}
		if (this.#completing) return;

		const deltaX = this.#normalizeDelta(event.deltaX, event.deltaMode);
		const deltaY = this.#normalizeDelta(event.deltaY, event.deltaMode);
		const horizontal = Math.abs(deltaX);
		const vertical = Math.abs(deltaY);

		if (this.#direction === 0) {
			// A slight horizontal lead is enough to begin. Once locked, vertical
			// trackpad noise does not make the gesture repeatedly start and stop.
			if (horizontal < 1 || horizontal <= vertical * 1.08) return;
			this.#direction = Math.sign(deltaX) || 1;
		}

		if (horizontal < 0.35 || horizontal < vertical * 0.45) return;
		// Once this is clearly our horizontal gesture, keep the webview's native
		// history/overscroll handling from animating underneath it. Competing
		// compositor gestures can leave visual and pointer coordinates out of sync.
		event.preventDefault?.();

		const directedDelta = deltaX * this.#direction;
		this.#distance = Math.max(
			0,
			this.#distance + (directedDelta >= 0 ? directedDelta : directedDelta * 1.7)
		);

		const threshold = this.#clamp(window.innerWidth * 0.2, 180, 300) * 0.72;
		const rawProgress = this.#clamp(this.#distance / threshold, 0, 1);
		// The edge responds immediately, then gains resistance near completion.
		this.#targetProgress = 1 - Math.pow(1 - rawProgress, 2.15);
		this.#animate();

		if (this.#releaseTimer) clearTimeout(this.#releaseTimer);
		this.#releaseTimer = setTimeout(() => this.#release(), 240);

		if (rawProgress >= 1) this.#complete();
	};

	destroy(): void {
		if (this.#frame != null) cancelAnimationFrame(this.#frame);
		if (this.#releaseTimer) clearTimeout(this.#releaseTimer);
		if (this.#completeTimer) clearTimeout(this.#completeTimer);
		this.#frame = null;
		this.#releaseTimer = null;
		this.#completeTimer = null;
	}

	#normalizeDelta(value: number, mode = 0): number {
		if (mode === 1) return value * 16;
		if (mode === 2) return value * window.innerWidth;
		return value;
	}

	#release(): void {
		if (this.#completing) return;
		this.#distance = 0;
		this.#direction = 0;
		this.#targetProgress = 0;
		this.#releaseTimer = null;
		this.#animate();
	}

	#complete(): void {
		this.#completing = true;
		this.#targetProgress = 1;
		if (this.#releaseTimer) clearTimeout(this.#releaseTimer);
		this.#releaseTimer = null;
		this.#animate();
		// Let the edge visibly finish its pull before changing the route.
		this.#completeTimer = setTimeout(() => this.#onComplete(), 260);
	}

	#animate(): void {
		if (this.#frame != null) return;

		const tick = () => {
			const difference = this.#targetProgress - this.progress;
			this.progress += difference * (this.#targetProgress > this.progress ? 0.24 : 0.16);

			if (Math.abs(difference) < 0.002) {
				this.progress = this.#targetProgress;
				this.#frame = null;
				return;
			}

			this.#frame = requestAnimationFrame(tick);
		};

		this.#frame = requestAnimationFrame(tick);
	}

	#clamp(value: number, minimum: number, maximum: number): number {
		return Math.min(maximum, Math.max(minimum, value));
	}
}
