//! Port of SyncSessionService — per-sync bookkeeping rows that let the last
//! process-mailbox job trigger finalization, plus stale-session cleanup.

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
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO sync_sessions (id, ingestion_source_id, is_initial_import, total_mailboxes, \
         completed_mailboxes, failed_mailboxes, error_messages) VALUES (?, ?, ?, ?, 0, 0, '[]')",
        rusqlite::params![id, ingestion_source_id, is_initial_import, total_mailboxes],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn find_by_id(conn: &Connection, session_id: &str) -> Result<SessionRecord, String> {
    conn.query_row(
        "SELECT ingestion_source_id, is_initial_import, total_mailboxes, completed_mailboxes, \
         failed_mailboxes, error_messages FROM sync_sessions WHERE id = ?",
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
    .map_err(|_| format!("Sync session {session_id} not found."))
}

/// recordMailboxResult — atomic counters + optional syncState json_patch merge.
pub fn record_mailbox_result(
    conn: &Connection,
    session_id: &str,
    result: Result<&Value, &str>, // Ok(syncState) | Err(errorMessage)
) -> Result<MailboxResultOutcome, String> {
    let now = crate::search::now_ms();
    match result {
        Ok(_) => conn.execute(
            "UPDATE sync_sessions SET completed_mailboxes = completed_mailboxes + 1, \
             last_activity_at = ? WHERE id = ?",
            rusqlite::params![now, session_id],
        ),
        Err(message) => conn.execute(
            "UPDATE sync_sessions SET failed_mailboxes = failed_mailboxes + 1, \
             error_messages = json_insert(error_messages, '$[#]', ?), last_activity_at = ? \
             WHERE id = ?",
            rusqlite::params![message, now, session_id],
        ),
    }
    .map_err(|e| e.to_string())?;

    let session = find_by_id(conn, session_id)?;

    if let Ok(sync_state) = result {
        if sync_state.as_object().map_or(false, |o| !o.is_empty()) {
            conn.execute(
                "UPDATE ingestion_sources SET sync_state = \
                 json_patch(COALESCE(sync_state, '{}'), ?) WHERE id = ?",
                rusqlite::params![sync_state.to_string(), session.ingestion_source_id],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let processed = session.completed_mailboxes + session.failed_mailboxes;
    Ok(MailboxResultOutcome { is_last: processed >= session.total_mailboxes })
}

pub fn heartbeat(conn: &Connection, session_id: &str) {
    conn.execute(
        "UPDATE sync_sessions SET last_activity_at = ? WHERE id = ?",
        rusqlite::params![crate::search::now_ms(), session_id],
    )
    .ok();
}

pub fn finalize(conn: &Connection, session_id: &str) {
    conn.execute("DELETE FROM sync_sessions WHERE id = ?", [session_id]).ok();
}

/// cleanStaleSessions — sessions silent for 30min: finished ones are removed
/// silently; genuinely stuck ones flip their source to 'error' and are removed.
pub fn clean_stale_sessions(conn: &Connection) {
    let cutoff = crate::search::now_ms() - 30 * 60 * 1000;
    let mut stmt = match conn.prepare(
        "SELECT id, ingestion_source_id, total_mailboxes, completed_mailboxes, failed_mailboxes \
         FROM sync_sessions WHERE last_activity_at < ?",
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
                 last_sync_status_message = 'Sync session stalled and was cleaned up.', \
                 updated_at = ? WHERE id = ?",
                rusqlite::params![crate::search::now_ms(), source_id],
            )
            .ok();
        }
        conn.execute("DELETE FROM sync_sessions WHERE id = ?", [id]).ok();
    }
}
