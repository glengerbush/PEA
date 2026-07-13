//! Fresh-data-dir provisioning: run schema migrations, set up FTS, and create
//! the storage dir. Single-user local app: no user record. Applied migrations
//! are tracked in the `__drizzle_migrations` table by sha256 hash, in journal
//! `when` order.

use include_dir::{include_dir, Dir};
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

static MIGRATIONS: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/migrations");

/// Apply pending migrations, recording each in `__drizzle_migrations` by
/// sha256 hash in journal `when` order.
pub fn run_migrations(conn: &Connection) -> Result<usize, String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS \"__drizzle_migrations\" (\n\
         \tid SERIAL PRIMARY KEY,\n\
         \thash text NOT NULL,\n\
         \tcreated_at numeric\n\
         )",
    )
    .map_err(|e| e.to_string())?;

    let last_created_at: Option<i64> = conn
        .query_row(
            "SELECT created_at FROM \"__drizzle_migrations\" ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();

    let journal: Value = serde_json::from_str(
        MIGRATIONS
            .get_file("meta/_journal.json")
            .ok_or("embedded migrations missing meta/_journal.json")?
            .contents_utf8()
            .ok_or("journal not utf8")?,
    )
    .map_err(|e| e.to_string())?;
    let entries = journal
        .get("entries")
        .and_then(|e| e.as_array())
        .ok_or("journal has no entries")?;

    let mut applied = 0usize;
    conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let result = (|| -> Result<usize, String> {
        for entry in entries {
            let tag = entry.get("tag").and_then(|t| t.as_str()).ok_or("entry missing tag")?;
            let when = entry.get("when").and_then(|w| w.as_i64()).ok_or("entry missing when")?;
            if last_created_at.map_or(false, |last| last >= when) {
                continue;
            }
            let sql = MIGRATIONS
                .get_file(format!("{tag}.sql"))
                .ok_or_else(|| format!("missing migration file {tag}.sql"))?
                .contents_utf8()
                .ok_or("migration not utf8")?;
            for stmt in sql.split("--> statement-breakpoint") {
                conn.execute_batch(stmt).map_err(|e| format!("{tag}: {e}"))?;
            }
            let hash = crate::hex_encode(Sha256::digest(sql.as_bytes()));
            conn.execute(
                "INSERT INTO \"__drizzle_migrations\" (\"hash\", \"created_at\") VALUES (?, ?)",
                rusqlite::params![hash, when],
            )
            .map_err(|e| e.to_string())?;
            applied += 1;
        }
        Ok(applied)
    })();
    match result {
        Ok(n) => {
            conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
            Ok(n)
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK").ok();
            Err(e)
        }
    }
}

/// ensureReady — FTS5 DDL + orphan sweep.
pub fn ensure_fts(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS email_fts USING fts5( \
            email_id UNINDEXED, subject, body, sender, recipients, attachments, meta, \
            tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3' \
        );",
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM email_fts WHERE email_id NOT IN (SELECT id FROM archived_emails)",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Full fresh-dir provisioning: schema, FTS, storage dir.
pub fn provision(data_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(data_dir.join("storage")).map_err(|e| e.to_string())?;
    let conn = Connection::open(data_dir.join("archive.db")).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "busy_timeout", 5000).ok();
    // Migrations run with FK enforcement OFF so a table-rebuild migration
    // (recreate-and-copy, to drop a column/FK) can't trip cascade deletes.
    // Normal pool connections enforce foreign keys (state.rs).
    conn.pragma_update(None, "foreign_keys", "OFF").ok();
    run_migrations(&conn)?;
    ensure_fts(&conn)?;
    // One-time backfill of sent_at_kind for rows imported before migration 0015.
    // Idempotent: after the first pass there are no NULL-kind rows to process.
    match crate::ingest::backfill_sent_at_kind(&conn, data_dir) {
        Ok(0) => {}
        Ok(n) => eprintln!("[provision] backfilled sent_at_kind for {n} email(s)"),
        Err(e) => eprintln!("[provision] sent_at_kind backfill failed (non-fatal): {e}"),
    }
    Ok(())
}
