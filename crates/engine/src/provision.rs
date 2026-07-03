//! Fresh-data-dir provisioning — the Rust twin of embedded.ts + drizzle's
//! migrator + SearchService.ensureReady + UserService.getOrCreateLocalUser.
//! Drizzle bookkeeping is reproduced exactly (same "__drizzle_migrations"
//! table, same sha256 hashes, same journal `when` ordering) so the Node and
//! Rust engines can run migrations interchangeably on the same archive.db.

use include_dir::{include_dir, Dir};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

static MIGRATIONS: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/migrations");

/// loadOrCreateSecrets — existing values are NEVER regenerated; unknown keys
/// are preserved untouched. File mode 0600.
pub fn ensure_secrets(data_dir: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let file = data_dir.join("secrets.json");
    let mut existing: Value = if file.exists() {
        serde_json::from_str(&std::fs::read_to_string(&file).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
    } else {
        json!({})
    };
    let obj = existing.as_object_mut().ok_or("secrets.json is not an object")?;
    for key in ["encryptionKey", "storageEncryptionKey"] {
        let missing = obj.get(key).and_then(|v| v.as_str()).map_or(true, str::is_empty);
        if missing {
            let bytes: [u8; 32] = std::array::from_fn(|_| rand::random());
            obj.insert(key.into(), json!(hex::encode(bytes)));
        }
    }
    let rendered = serde_json::to_string_pretty(&existing)
        .map_err(|e| e.to_string())?
        .replace("  ", "\t"); // match Node's tab indentation closely enough
    std::fs::write(&file, format!("{rendered}\n")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(existing)
}

/// drizzle-orm better-sqlite3 migrate(): identical table, hash, and ordering.
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
            let hash = hex::encode(Sha256::digest(sql.as_bytes()));
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

/// SearchService.ensureReady — FTS5 DDL + orphan sweep.
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

/// UserService.getOrCreateLocalUser — first user wins; placeholder otherwise.
pub fn ensure_local_user(conn: &Connection) -> Result<String, String> {
    if let Ok(id) = conn.query_row(
        "SELECT id FROM users ORDER BY created_at ASC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    ) {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let email = std::env::var("ADMIN_EMAIL").unwrap_or_else(|_| "local@localhost".into());
    conn.execute(
        "INSERT INTO users (id, email, first_name, last_name) VALUES (?, ?, 'Local', 'User')",
        rusqlite::params![id, email],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Full fresh-dir provisioning: secrets, schema, FTS, local user, storage dir.
pub fn provision(data_dir: &Path) -> Result<(), String> {
    ensure_secrets(data_dir)?;
    std::fs::create_dir_all(data_dir.join("storage")).map_err(|e| e.to_string())?;
    let conn = Connection::open(data_dir.join("archive.db")).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "busy_timeout", 5000).ok();
    conn.pragma_update(None, "foreign_keys", "ON").ok();
    run_migrations(&conn)?;
    ensure_fts(&conn)?;
    ensure_local_user(&conn)?;
    Ok(())
}
