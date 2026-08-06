export const SNAPSHOT_SETTLE_DELAY_MS = 16;

export class SwipeBackSnapshotScheduler {
	#timer: ReturnType<typeof setTimeout> | null = null;

	schedule(capture: () => void, delay = SNAPSHOT_SETTLE_DELAY_MS): void {
		this.cancel();
		this.#timer = setTimeout(() => {
			this.#timer = null;
			capture();
		}, delay);
	}

	cancel(): void {
		if (this.#timer) clearTimeout(this.#timer);
		this.#timer = null;
	}
}
