export const SNAPSHOT_QUIET_PERIOD_MS = 700;

export class SwipeBackSnapshotScheduler {
	#timer: ReturnType<typeof setTimeout> | null = null;

	schedule(capture: () => void, delay = SNAPSHOT_QUIET_PERIOD_MS): void {
		this.cancel();
		this.#timer = setTimeout(() => {
			this.#timer = null;
			capture();
		}, delay);
	}

	deferIfPending(capture: () => void): void {
		if (!this.#timer) return;
		this.schedule(capture);
	}

	cancel(): void {
		if (this.#timer) clearTimeout(this.#timer);
		this.#timer = null;
	}
}
