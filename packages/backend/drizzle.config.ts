import { defineConfig } from 'drizzle-kit';
import path from 'path';
import os from 'os';
import { config } from 'dotenv';

config();

// Mirrors src/data-dir.ts (drizzle-kit can't import TS from src).
const dataDir =
	process.env.OA_DATA_DIR ||
	(process.platform === 'darwin'
		? path.join(os.homedir(), 'Library', 'Application Support', 'OpenArchiver')
		: path.join(
				process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share'),
				'open-archiver'
			));
const dbPath = process.env.OA_DB_PATH || path.join(dataDir, 'archive.db');

export default defineConfig({
	schema: './src/database/schema.ts',
	out: './src/database/migrations',
	dialect: 'sqlite',
	dbCredentials: {
		url: dbPath,
	},
	verbose: true,
	strict: true,
});
