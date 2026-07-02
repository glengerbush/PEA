import { migrate } from 'drizzle-orm/better-sqlite3/migrator';
import path from 'path';
import { config } from 'dotenv';
import { db } from './index';

config();

// Resolved relative to this file (dist/database at runtime, src/database under
// ts-node) instead of cwd. `copy-assets` places the SQL migrations at
// dist/database/migrations; the desktop bundle sets OA_MIGRATIONS_DIR.
const migrationsFolder = process.env.OA_MIGRATIONS_DIR || path.resolve(__dirname, 'migrations');

/** Runs pending migrations against the app's SQLite database. */
export const runMigrations = async (): Promise<void> => {
	migrate(db, { migrationsFolder });
};

// CLI entry: `node dist/database/migrate.js` (pnpm db:migrate)
if (require.main === module) {
	runMigrations()
		.then(() => {
			console.log('Migrations completed!');
			process.exit(0);
		})
		.catch((err) => {
			console.error('Migration failed!', err);
			process.exit(1);
		});
}
