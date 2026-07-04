/**
 * Result of the desktop app's Tauri release-updater check
 * (`GET /api/v1/native/update-check`). This reflects the actual installable,
 * signed build on GitHub Releases — the mechanism that can self-install.
 */
export interface NativeUpdateInfo {
	/** A newer signed release is available to install. */
	available: boolean;
	/** The version the app is currently running. */
	currentVersion: string;
	/** The available release version, when `available` is true. */
	version?: string | null;
	/** Release notes (the GitHub release body), when provided. */
	notes?: string | null;
	/** Link to the project's releases page. */
	releasesUrl?: string;
	/**
	 * Present when the check couldn't run — e.g. the updater is unavailable
	 * outside the packaged desktop app, or the network request failed.
	 */
	error?: string;
}
