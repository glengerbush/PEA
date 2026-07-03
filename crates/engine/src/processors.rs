//! Job processors — ports of jobs/processors/*.processor.ts, dispatched by
//! (queue, job name) exactly like the Node makeDispatcher maps.

use crate::state::AppState;
use crate::{connectors, ingest, queue, search, sessions, sources};
use serde_json::{json, Value};

pub fn dispatch(state: &AppState, queue_name: &str, name: &str, payload: &Value) -> Result<(), String> {
    match (queue_name, name) {
        ("ingestion", "initial-import") => initial_import(state, payload),
        ("ingestion", "process-mailbox") => process_mailbox(state, payload),
        ("ingestion", "sync-cycle-finished") => sync_cycle_finished(state, payload),
        ("ingestion", "continuous-sync") => continuous_sync(state, payload),
        ("ingestion", "schedule-continuous-sync") => schedule_continuous_sync(state, payload),
        ("indexing", "index-email-batch") => index_email_batch(state, payload),
        ("indexing", "scan-fuzzy-duplicates") => scan_fuzzy_duplicates(state, payload),
        ("remote-content", "archive-remote-content-batch") => {
            crate::remote_content::archive_batch(state, payload)
        }
        _ => Err(format!("Unknown job name: {name}")),
    }
}

fn source_id_of(payload: &Value) -> Result<String, String> {
    payload
        .get("ingestionSourceId")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "missing ingestionSourceId".into())
}

/// Mailbox users for a source — the connector's listAllUsers.
fn list_users(source: &sources::SourceRow) -> Result<Vec<String>, String> {
    match source.provider.as_str() {
        "mbox_import" => Ok(vec![connectors::mbox_user_email(&source.credentials)]),
        "eml_import" => Ok(vec![crate::eml::eml_user_email(&source.credentials)]),
        other => Err(format!("Unsupported provider: {other}")),
    }
}

fn initial_import(state: &AppState, payload: &Value) -> Result<(), String> {
    let source_id = source_id_of(payload)?;
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let source = sources::find_by_id(state, &conn, &source_id)?;
        sources::update_source(
            state,
            &conn,
            &source_id,
            &json!({ "status": "importing", "lastSyncStatusMessage": "Starting initial import..." }),
        )?;
        let users = list_users(&source)?;
        if users.is_empty() {
            let status = if sources::FILE_BASED_PROVIDERS.contains(&source.provider.as_str()) {
                "imported"
            } else {
                "active"
            };
            sources::update_source(
                state,
                &conn,
                &source_id,
                &json!({
                    "status": status,
                    "lastSyncFinishedAt": true,
                    "lastSyncStatusMessage": "Initial import complete. No users found.",
                }),
            )?;
            return Ok(());
        }
        let session_id = sessions::create(&conn, &source_id, users.len() as i64, true)?;
        for user_email in users {
            queue::send_job(
                state,
                "ingestion",
                "process-mailbox",
                &json!({
                    "ingestionSourceId": source_id,
                    "userEmail": user_email,
                    "sessionId": session_id,
                }),
                queue::SendOptions::default(),
            );
        }
        Ok(())
    })();
    if let Err(e) = &result {
        sources::update_source(
            state,
            &conn,
            &source_id,
            &json!({ "status": "error", "lastSyncStatusMessage": format!("Initial import failed: {e}") }),
        )
        .ok();
    }
    result
}

fn process_mailbox(state: &AppState, payload: &Value) -> Result<(), String> {
    let source_id = source_id_of(payload)?;
    let user_email = payload
        .get("userEmail")
        .and_then(|v| v.as_str())
        .ok_or("missing userEmail")?
        .to_string();
    let session_id = payload
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or("missing sessionId")?
        .to_string();
    let batch_size: usize = std::env::var("PEA_INDEXING_BATCH")
        .or_else(|_| std::env::var("OA_INDEXING_BATCH"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);

    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let run = (|| -> Result<(), String> {
        let source = sources::find_by_id(state, &conn, &source_id)?;
        // Child sources are assistants: content ownership goes to the root.
        let effective = match &source.merged_into_id {
            Some(root_id) => sources::find_by_id(state, &conn, root_id)?,
            None => sources::find_by_id(state, &conn, &source_id)?,
        };
        let group_ids = search::group_source_ids(&conn, &source_id)
            .ok_or("Ingestion source not found")?;

        // Collect archived ids, then flush in indexing-batch-sized chunks
        // (Node flushes mid-stream; the resulting job set is identical).
        let mut pending: Vec<String> = Vec::new();
        {
            let handler = |email: ingest::EmailObj| {
                match ingest::process_email(
                    state, &conn, &source_id, &group_ids, &effective, &email, &user_email,
                ) {
                    Ok(Some(id)) => pending.push(id),
                    Ok(None) => {}
                    Err(e) => eprintln!("[ingest] failed to process email {}: {e}", email.id),
                }
            };
            match source.provider.as_str() {
                "mbox_import" => connectors::for_each_email(state, &source.credentials, handler)?,
                "eml_import" => crate::eml::for_each_email(state, &source.credentials, handler)?,
                other => return Err(format!("Unsupported provider: {other}")),
            }
        }
        for chunk in pending.chunks(batch_size.max(1)) {
            let emails: Vec<Value> =
                chunk.iter().map(|id| json!({ "archivedEmailId": id })).collect();
            queue::send_job(
                state,
                "indexing",
                "index-email-batch",
                &json!({ "emails": emails }),
                queue::SendOptions::default(),
            );
            crate::remote_content::enqueue_archive(state, chunk);
            sessions::heartbeat(&conn, &session_id);
        }
        Ok(())
    })();

    match run {
        Ok(()) => {
            let outcome = sessions::record_mailbox_result(&conn, &session_id, Ok(&json!({})))?;
            if outcome.is_last {
                let session = sessions::find_by_id(&conn, &session_id)?;
                queue::send_job(
                    state,
                    "ingestion",
                    "sync-cycle-finished",
                    &json!({
                        "ingestionSourceId": source_id,
                        "sessionId": session_id,
                        "isInitialImport": session.is_initial_import,
                    }),
                    queue::SendOptions::default(),
                );
            }
            Ok(())
        }
        Err(message) => {
            // Node wraps the connector failure with the mailbox context.
            let message = format!("Failed to process mailbox for {user_email}: {message}");
            let outcome = sessions::record_mailbox_result(&conn, &session_id, Err(&message))?;
            if outcome.is_last {
                let session = sessions::find_by_id(&conn, &session_id)?;
                queue::send_job(
                    state,
                    "ingestion",
                    "sync-cycle-finished",
                    &json!({
                        "ingestionSourceId": source_id,
                        "sessionId": session_id,
                        "isInitialImport": session.is_initial_import,
                    }),
                    queue::SendOptions::default(),
                );
            }
            Err(message)
        }
    }
}

fn sync_cycle_finished(state: &AppState, payload: &Value) -> Result<(), String> {
    let source_id = source_id_of(payload)?;
    let session_id = payload
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or("missing sessionId")?;
    let is_initial = payload
        .get("isInitialImport")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let session = sessions::find_by_id(&conn, session_id)?;
    let source = sources::find_by_id(state, &conn, &source_id)?;

    let mut status = "active";
    if sources::FILE_BASED_PROVIDERS.contains(&source.provider.as_str()) {
        status = "imported";
    }
    let message = if session.failed_mailboxes > 0 {
        status = "error";
        format!(
            "Sync cycle completed with {} error(s):\n{}",
            session.failed_mailboxes,
            session.error_messages.join("\n")
        )
    } else if is_initial || session.is_initial_import {
        format!("Import finished. Archived {} mailbox(es).", session.completed_mailboxes)
    } else {
        "Import completed successfully.".to_string()
    };
    let final_status = if source.status == "paused" { "paused" } else { status };
    sources::update_source(
        state,
        &conn,
        &source_id,
        &json!({
            "status": final_status,
            "lastSyncFinishedAt": true,
            "lastSyncStatusMessage": message,
        }),
    )?;
    sessions::finalize(&conn, session_id);
    Ok(())
}

fn continuous_sync(state: &AppState, payload: &Value) -> Result<(), String> {
    let source_id = source_id_of(payload)?;
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let source = sources::find_by_id(state, &conn, &source_id)?;
    if source.status != "active" && source.status != "error" {
        return Ok(()); // skip non-active/error sources
    }
    sources::update_source(
        state,
        &conn,
        &source_id,
        &json!({ "status": "syncing", "lastSyncStartedAt": true }),
    )?;
    let users = list_users(&source)?;
    if users.is_empty() {
        sources::update_source(
            state,
            &conn,
            &source_id,
            &json!({
                "status": "active",
                "lastSyncFinishedAt": true,
                "lastSyncStatusMessage": "Continuous sync complete. No users found.",
            }),
        )?;
        return Ok(());
    }
    let session_id = sessions::create(&conn, &source_id, users.len() as i64, false)?;
    for user_email in users {
        queue::send_job(
            state,
            "ingestion",
            "process-mailbox",
            &json!({
                "ingestionSourceId": source.id,
                "userEmail": user_email,
                "sessionId": session_id,
            }),
            queue::SendOptions::default(),
        );
    }
    Ok(())
}

fn schedule_continuous_sync(state: &AppState, _payload: &Value) -> Result<(), String> {
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    sessions::clean_stale_sessions(&conn);
    let mut stmt = conn
        .prepare("SELECT id FROM ingestion_sources WHERE status IN ('active', 'error')")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    for id in ids {
        queue::send_job(
            state,
            "ingestion",
            "continuous-sync",
            &json!({ "ingestionSourceId": id }),
            queue::master_job_options(&format!("continuous-sync:{id}")),
        );
    }
    Ok(())
}

fn index_email_batch(state: &AppState, payload: &Value) -> Result<(), String> {
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    crate::provision::ensure_fts(&conn)?;
    let emails = payload
        .get("emails")
        .and_then(|v| v.as_array())
        .ok_or("missing emails")?;
    for entry in emails {
        let Some(id) = entry.get("archivedEmailId").and_then(|v| v.as_str()) else { continue };
        if let Err(e) = ingest::index_email(state, &conn, id) {
            eprintln!("[index] failed to index email {id}: {e}");
        }
    }
    Ok(())
}

/// DuplicateReviewService.scanFuzzyDuplicateBatch — score candidates (read),
/// then upsert groups + link emails (write) in one transaction.
fn scan_fuzzy_duplicates(state: &AppState, payload: &Value) -> Result<(), String> {
    let batch_size = payload
        .get("batchSize")
        .and_then(|v| v.as_i64())
        .filter(|n| *n >= 1)
        .map(|n| n.min(500))
        .unwrap_or(100);
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let window_ms: i64 = 48 * 3600 * 1000;
    let score_expr = format!(
        "(45 \
         + CASE WHEN body_hash_present_count = email_count AND body_hash_count = 1 THEN 20 ELSE 0 END \
         + CASE WHEN recipient_hash_present_count = email_count AND recipient_hash_count = 1 THEN 15 ELSE 0 END \
         + CASE WHEN attachment_hash_present_count = email_count AND attachment_hash_count = 1 THEN 10 ELSE 0 END \
         + CASE WHEN max_sent_at - min_sent_at <= {window_ms} THEN 10 ELSE 0 END)"
    );
    let sql = format!(
        "WITH candidate_base AS ( \
            SELECT ae.duplicate_fuzzy_group_key AS group_key, \
                min(lower(ae.sender_email)) AS sender_email, \
                min(ae.duplicate_subject_hash) AS duplicate_subject_hash, \
                count(*) AS email_count, \
                min(ae.sent_at) AS min_sent_at, \
                max(ae.sent_at) AS max_sent_at, \
                count(ae.duplicate_body_hash) AS body_hash_present_count, \
                count(DISTINCT ae.duplicate_body_hash) AS body_hash_count, \
                count(ae.duplicate_recipient_fingerprint) AS recipient_hash_present_count, \
                count(DISTINCT ae.duplicate_recipient_fingerprint) AS recipient_hash_count, \
                count(ae.duplicate_attachment_fingerprint) AS attachment_hash_present_count, \
                count(DISTINCT ae.duplicate_attachment_fingerprint) AS attachment_hash_count \
            FROM archived_emails ae \
            WHERE ae.duplicate_fuzzy_group_key IS NOT NULL \
                AND NOT EXISTS ( \
                    SELECT 1 FROM fuzzy_duplicate_groups fdg \
                    WHERE fdg.group_key = ae.duplicate_fuzzy_group_key \
                        AND fdg.status IN ('approved', 'ignored') \
                ) \
            GROUP BY ae.duplicate_fuzzy_group_key \
            HAVING count(*) > 1 \
        ) \
        SELECT *, {score_expr} AS score FROM candidate_base \
        WHERE {score_expr} >= 55 \
        ORDER BY score DESC, email_count DESC, group_key ASC LIMIT {batch_size}"
    );
    struct Candidate {
        group_key: String,
        sender_email: Option<String>,
        subject_hash: Option<String>,
        #[allow(dead_code)]
        email_count: i64,
        min_sent_at: i64,
        max_sent_at: i64,
        body_all: bool,
        recipients_all: bool,
        attachments_all: bool,
        score: i64,
    }
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let candidates: Vec<Candidate> = stmt
        .query_map([], |row| {
            let email_count: i64 = row.get("email_count")?;
            let body_present: i64 = row.get("body_hash_present_count")?;
            let body_distinct: i64 = row.get("body_hash_count")?;
            let rcpt_present: i64 = row.get("recipient_hash_present_count")?;
            let rcpt_distinct: i64 = row.get("recipient_hash_count")?;
            let att_present: i64 = row.get("attachment_hash_present_count")?;
            let att_distinct: i64 = row.get("attachment_hash_count")?;
            Ok(Candidate {
                group_key: row.get("group_key")?,
                sender_email: row.get("sender_email")?,
                subject_hash: row.get("duplicate_subject_hash")?,
                email_count,
                min_sent_at: row.get("min_sent_at")?,
                max_sent_at: row.get("max_sent_at")?,
                body_all: body_present == email_count && body_distinct == 1,
                recipients_all: rcpt_present == email_count && rcpt_distinct == 1,
                attachments_all: att_present == email_count && att_distinct == 1,
                score: row.get("score")?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let tx_result = (|| -> Result<(), String> {
        for c in &candidates {
            let existing: Option<(String, String)> = conn
                .query_row(
                    "SELECT id, status FROM fuzzy_duplicate_groups WHERE group_key = ?",
                    [&c.group_key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            if let Some((_, status)) = &existing {
                if status != "pending" {
                    continue;
                }
            }
            let signals = json!({
                "senderEmail": c.sender_email,
                "subjectHash": c.subject_hash,
                "matchingBodyHash": c.body_all,
                "matchingRecipients": c.recipients_all,
                "matchingAttachments": c.attachments_all,
                "sentSpreadHours": (c.max_sent_at - c.min_sent_at) as f64 / 3_600_000.0,
            });
            let group_id = match existing {
                Some((id, _)) => {
                    conn.execute(
                        "UPDATE fuzzy_duplicate_groups SET score = ?, signals = ?, updated_at = ? WHERE id = ?",
                        rusqlite::params![c.score, signals.to_string(), crate::search::now_ms(), id],
                    )
                    .map_err(|e| e.to_string())?;
                    id
                }
                None => {
                    let id = uuid::Uuid::new_v4().to_string();
                    conn.execute(
                        "INSERT INTO fuzzy_duplicate_groups (id, group_key, status, score, signals) \
                         VALUES (?, ?, 'pending', ?, ?)",
                        rusqlite::params![id, c.group_key, c.score, signals.to_string()],
                    )
                    .map_err(|e| e.to_string())?;
                    id
                }
            };
            conn.execute(
                "INSERT OR IGNORE INTO fuzzy_duplicate_group_emails (group_id, email_id, suggested_keeper) \
                 SELECT ?, ae.id, ae.id = ( \
                     SELECT keeper.id FROM archived_emails keeper \
                     WHERE keeper.duplicate_fuzzy_group_key = ? \
                     ORDER BY keeper.sent_at ASC, keeper.archived_at ASC, keeper.id ASC LIMIT 1) \
                 FROM archived_emails ae WHERE ae.duplicate_fuzzy_group_key = ?",
                rusqlite::params![group_id, c.group_key, c.group_key],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    match tx_result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(|e| e.to_string()),
        Err(e) => {
            conn.execute_batch("ROLLBACK").ok();
            Err(e)
        }
    }
}
