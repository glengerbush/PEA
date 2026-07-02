import path from 'path';
import os from 'os';

/**
 * OS-conventional app-data location, overridable via OA_DATA_DIR.
 * Dependency-free: imported by both the embedded runtime and the database
 * layer (which resolves the SQLite file at module load).
 */
export const resolveDataDir = (): string => {
	if (process.env.OA_DATA_DIR) {
		return path.resolve(process.env.OA_DATA_DIR);
	}
	if (process.platform === 'darwin') {
		return path.join(os.homedir(), 'Library', 'Application Support', 'OpenArchiver');
	}
	const xdgData = process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share');
	return path.join(xdgData, 'open-archiver');
};

/** The SQLite database file (the whole archive index lives in this one file). */
export const resolveDbPath = (): string =>
	process.env.OA_DB_PATH || path.join(resolveDataDir(), 'archive.db');
