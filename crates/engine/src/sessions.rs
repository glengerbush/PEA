//! Per-import-run bookkeeping rows (import_sessions) that let the last
//! process-mailbox job trigger finalization, plus stale-run cleanup.

use rusqlite::Connection;
use serde_json::Value;

pub struct SessionRecord {
    pub ingestion_source_id: String,
    pub is_initial_import: bool,
    pub total_mailboxes: i64,
    pub completed_mailboxes: i64,
    pub failed_mailboxes: i64,
    pub error_messages: Vec<String>,
}

pub struct MailboxResultOutcome {
    pub is_last: bool,
}

pub fn create(
    conn: &Connection,
    ingestion_source_id: &str,
    total_mailboxes: i64,
    is_initial_import: bool,
    total_bytes: i64,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO import_sessions (id, ingestion_source_id, is_initial_import, total_mailboxes, \
         completed_mailboxes, failed_mailboxes, error_messages, total_bytes, processed_bytes) \
         VALUES (?, ?, ?, ?, 0, 0, '[]', ?, 0)",
        rusqlite::params![id, ingestion_source_id, is_initial_import, total_mailboxes, total_bytes],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Adds streamed input bytes to the session's progress counter and refreshes
/// the heartbeat. Deltas (not absolute writes) stay correct if a session ever
/// has more than one concurrent mailbox worker.
pub fn add_progress(conn: &Connection, session_id: &str, delta_bytes: i64) {
    conn.execute(
        "UPDATE import_sessions SET processed_bytes = processed_bytes + ?, last_activity_at = ? \
         WHERE id = ?",
        rusqlite::params![delta_bytes, crate::search::now_ms(), session_id],
    )
    .ok();
}

pub fn find_by_id(conn: &Connection, session_id: &str) -> Result<SessionRecord, String> {
    conn.query_row(
        "SELECT ingestion_source_id, is_initial_import, total_mailboxes, completed_mailboxes, \
         failed_mailboxes, error_messages FROM import_sessions WHERE id = ?",
        [session_id],
        |row| {
            Ok(SessionRecord {
                ingestion_source_id: row.get(0)?,
                is_initial_import: row.get::<_, i64>(1)? != 0,
                total_mailboxes: row.get(2)?,
                completed_mailboxes: row.get(3)?,
                failed_mailboxes: row.get(4)?,
                error_messages: serde_json::from_str(&row.get::<_, String>(5)?)
                    .unwrap_or_default(),
            })
        },
    )
    .map_err(|_| format!("Import session {session_id} not found."))
}

/// recordMailboxResult — atomic counters + optional state json_patch merge.
pub fn record_mailbox_result(
    conn: &Connection,
    session_id: &str,
    result: Result<&Value, &str>, // Ok(state) | Err(errorMessage)
) -> Result<MailboxResultOutcome, String> {
    let now = crate::search::now_ms();
    // Increment and read the post-increment counters in ONE statement (RETURNING)
    // so that, under concurrent mailbox completions, exactly one worker observes
    // is_last — SQLite serializes writers, so each sees its own incremented value.
    let (completed, failed, total): (i64, i64, i64) = match result {
        Ok(_) => conn.query_row(
            "UPDATE import_sessions SET completed_mailboxes = completed_mailboxes + 1, \
             last_activity_at = ? WHERE id = ? \
             RETURNING completed_mailboxes, failed_mailboxes, total_mailboxes",
            rusqlite::params![now, session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ),
        Err(message) => conn.query_row(
            "UPDATE import_sessions SET failed_mailboxes = failed_mailboxes + 1, \
             error_messages = json_insert(error_messages, '$[#]', ?), last_activity_at = ? \
             WHERE id = ? \
             RETURNING completed_mailboxes, failed_mailboxes, total_mailboxes",
            rusqlite::params![message, now, session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ),
    }
    .map_err(|e| e.to_string())?;

    let processed = completed + failed;
    Ok(MailboxResultOutcome { is_last: processed >= total })
}

pub fn heartbeat(conn: &Connection, session_id: &str) {
    conn.execute(
        "UPDATE import_sessions SET last_activity_at = ? WHERE id = ?",
        rusqlite::params![crate::search::now_ms(), session_id],
    )
    .ok();
}

pub fn finalize(conn: &Connection, session_id: &str) {
    conn.execute("DELETE FROM import_sessions WHERE id = ?", [session_id]).ok();
}

/// cleanStaleSessions — sessions silent for 30min: finished ones are removed
/// silently; genuinely stuck ones flip their source to 'error' and are removed.
pub fn clean_stale_sessions(conn: &Connection) {
    let cutoff = crate::search::now_ms() - 30 * 60 * 1000;
    let mut stmt = match conn.prepare(
        "SELECT id, ingestion_source_id, total_mailboxes, completed_mailboxes, failed_mailboxes \
         FROM import_sessions WHERE last_activity_at < ?",
    ) {
        Ok(s) => s,
        Err(_) => return,
    };
    let stale: Vec<(String, String, i64, i64, i64)> = stmt
        .query_map([cutoff], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    for (id, source_id, total, completed, failed) in stale {
        if completed + failed < total {
            conn.execute(
                "UPDATE ingestion_sources SET status = 'error', \
                 last_import_status_message = 'Import run stalled and was cleaned up.', \
                 updated_at = ? WHERE id = ?",
                rusqlite::params![crate::search::now_ms(), source_id],
            )
            .ok();
        }
        conn.execute("DELETE FROM import_sessions WHERE id = ?", [id]).ok();
    }
}
