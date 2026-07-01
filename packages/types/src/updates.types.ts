export interface UpdateCommit {
	sha: string;
	/** First line of the commit message. */
	message: string;
}

export type UpdateStatus = 'up_to_date' | 'behind' | 'unknown' | 'error';

export interface UpdateCheckResult {
	/** Full git SHA baked into the running build ('unknown' if the image wasn't stamped). */
	currentSha: string;
	/** Latest commit SHA on the tracked branch, or null if it couldn't be resolved. */
	latestSha: string | null;
	updateAvailable: boolean;
	/** How many commits the deployed build is behind the branch tip. */
	behindBy: number;
	/** New commits between the deployed build and the branch tip (oldest first). */
	commits: UpdateCommit[];
	/** GitHub compare URL (current...latest), or null. */
	compareUrl: string | null;
	/** ISO timestamp of when the check ran. */
	checkedAt: string;
	status: UpdateStatus;
	/** Human-readable note, e.g. why the status is 'unknown'. */
	message?: string;
	/** The command the user runs on the host to apply the update. */
	updateCommand: string;
}
