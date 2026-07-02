import Database, { type Database as SqliteDatabase } from 'better-sqlite3';
import { drizzle } from 'drizzle-orm/better-sqlite3';
import { mkdirSync } from 'fs';
import path from 'path';
import 'dotenv/config';

import * as schema from './schema';
import { resolveDbPath } from '../data-dir';

const dbPath = resolveDbPath();
mkdirSync(path.dirname(dbPath), { recursive: true });

const client = new Database(dbPath);
// WAL: readers never block the writer (the UI stays responsive during imports).
client.pragma('journal_mode = WAL');
client.pragma('synchronous = NORMAL'); // durable-enough with WAL, much faster
client.pragma('foreign_keys = ON'); // SQLite defaults them OFF per-connection
client.pragma('busy_timeout = 5000');

export const db = drizzle(client, { schema });

/** The raw better-sqlite3 handle (transactions, FTS maintenance, backups). */
export const sqlite: SqliteDatabase = client;

/** Closes the database (graceful shutdown). Sync, but kept Promise-shaped for callers. */
export const closeDb = async (): Promise<void> => {
	client.close();
};
