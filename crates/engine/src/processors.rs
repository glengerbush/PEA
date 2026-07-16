//! Job processors, dispatched by (queue, job name).

use crate::state::AppState;
use crate::{readers, ingest, queue, search, sessions, sources};
use serde_json::{json, Value};

pub fn dispatch(state: &AppState, queue_name: &str, name: &str, payload: &Value) -> Result<(), String> {
    match (queue_name, name) {
        ("ingestion", "initial-import") => initial_import(state, payload),
        ("ingestion", "process-mailbox") => process_mailbox(state, payload),
        ("ingestion", "import-cycle-finished") => import_cycle_finished(state, payload),
        // Reached via the Re-import action (re-runs an import; the dedupe
        // pass makes unchanged folders a no-op and merges new folder tags).
        ("ingestion", "reimport") => reimport_source(state, payload),
        ("indexing", "index-email-batch") => index_email_batch(state, payload),
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

/// The import sources for a source row — one entry per importable mailbox/file.
fn list_import_sources(source: &sources::SourceRow) -> Result<Vec<String>, String> {
    match source.provider.as_str() {
        "mbox_import" => Ok(vec![readers::mbox_import_source(&source.provider_config)]),
        "eml_import" => Ok(vec![crate::eml::eml_import_source(&source.provider_config)]),
        other => Err(format!("Unsupported provider: {other}")),
    }
}

/// Sum of the source's input file sizes, known before streaming starts — the
/// denominator for import progress percentages. 0 when enumeration fails
/// (progress is then simply not shown).
fn total_input_bytes(source: &sources::SourceRow) -> i64 {
    let paths: Vec<String> = match source.provider.as_str() {
        "mbox_import" => readers::get_mbox_inputs(&source.provider_config)
            .map(|inputs| inputs.into_iter().map(|i| i.file_path).collect())
            .unwrap_or_default(),
        "eml_import" => source
            .provider_config
            .get("localFilePath")
            .and_then(|v| v.as_str())
            .map(|p| vec![p.to_string()])
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    paths
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len() as i64)
        .sum()
}

fn initial_import(state: &AppState, payload: &Value) -> Result<(), String> {
    let source_id = source_id_of(payload)?;
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let source = sources::find_by_id(&conn, &source_id)?;
        sources::update_source(
            state,
            &conn,
            &source_id,
            &json!({ "status": "importing", "lastImportStatusMessage": "Starting initial import..." }),
        )?;
        let import_sources = list_import_sources(&source)?;
        if import_sources.is_empty() {
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
                    "lastImportFinishedAt": true,
                    "lastImportStatusMessage": "Initial import complete. No import sources found.",
                }),
            )?;
            return Ok(());
        }
        let session_id = sessions::create(
            &conn,
            &source_id,
            import_sources.len() as i64,
            true,
            total_input_bytes(&source),
        )?;
        for import_source in import_sources {
            queue::send_job(
                state,
                "ingestion",
                "process-mailbox",
                &json!({
                    "ingestionSourceId": source_id,
                    "importSource": import_source,
                    "sessionId": session_id,
                }),
                queue::no_retry(),
            );
        }
        Ok(())
    })();
    if let Err(e) = &result {
        sources::update_source(
            state,
            &conn,
            &source_id,
            &json!({ "status": "error", "lastImportStatusMessage": format!("Initial import failed: {e}") }),
        )
        .ok();
    }
    result
}

fn process_mailbox(state: &AppState, payload: &Value) -> Result<(), String> {
    let source_id = source_id_of(payload)?;
    let import_source = payload
        .get("importSource")
        .and_then(|v| v.as_str())
        .ok_or("missing importSource")?
        .to_string();
    let session_id = payload
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or("missing sessionId")?
        .to_string();
    let batch_size: usize = std::env::var("PEA_INDEXING_BATCH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);

    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let run = (|| -> Result<(), String> {
        let source = sources::find_by_id(&conn, &source_id)?;
        // Child sources are assistants: content ownership goes to the root.
        let effective = match &source.merged_into_id {
            Some(root_id) => sources::find_by_id(&conn, root_id)?,
            None => sources::find_by_id(&conn, &source_id)?,
        };
        let group_ids = search::group_source_ids(&conn, &source_id)
            .ok_or("Ingestion source not found")?;

        // Collect archived ids, then flush in indexing-batch-sized chunks.
        let mut pending: Vec<String> = Vec::new();
        {
            // Streaming a large mbox can take much longer than the 30-min
            // stale-session cutoff. Heartbeat during ingestion (not only after)
            // so clean_stale_sessions never falsely reaps a live import.
            let mut last_beat = std::time::Instant::now();
            let handler = |email: ingest::EmailObj| {
                match ingest::process_email(
                    state, &conn, &source_id, &group_ids, &effective, &email, &import_source,
                ) {
                    Ok(Some(id)) => pending.push(id),
                    Ok(None) => {}
                    Err(e) => eprintln!("[ingest] failed to process email {}: {e}", email.id),
                }
                if last_beat.elapsed().as_secs() >= 60 {
                    sessions::heartbeat(&conn, &session_id);
                    last_beat = std::time::Instant::now();
                }
            };
            // Byte-progress flushes, throttled to ~2s: the imports page polls at
            // 1s, so this keeps the percentage moving without a write per message.
            let mut unflushed: u64 = 0;
            let mut last_flush = std::time::Instant::now();
            let on_bytes = |delta: u64| {
                unflushed += delta;
                if last_flush.elapsed().as_secs() >= 2 {
                    sessions::add_progress(&conn, &session_id, unflushed as i64);
                    unflushed = 0;
                    last_flush = std::time::Instant::now();
                }
            };
            match source.provider.as_str() {
                "mbox_import" => {
                    readers::for_each_email(&source.provider_config, handler, on_bytes)?
                }
                "eml_import" => {
                    crate::eml::for_each_email(&source.provider_config, handler, on_bytes)?
                }
                other => return Err(format!("Unsupported provider: {other}")),
            }
            if unflushed > 0 {
                sessions::add_progress(&conn, &session_id, unflushed as i64);
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
                    "import-cycle-finished",
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
            // Wrap the reader failure with the mailbox context.
            let message = format!("Failed to process import {import_source}: {message}");
            let outcome = sessions::record_mailbox_result(&conn, &session_id, Err(&message))?;
            if outcome.is_last {
                let session = sessions::find_by_id(&conn, &session_id)?;
                queue::send_job(
                    state,
                    "ingestion",
                    "import-cycle-finished",
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

fn import_cycle_finished(state: &AppState, payload: &Value) -> Result<(), String> {
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
    let source = sources::find_by_id(&conn, &source_id)?;

    let mut status = "active";
    if sources::FILE_BASED_PROVIDERS.contains(&source.provider.as_str()) {
        status = "imported";
    }
    let message = if session.failed_mailboxes > 0 {
        status = "error";
        format!(
            "Import finished with {} error(s):\n{}",
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
            "lastImportFinishedAt": true,
            "lastImportStatusMessage": message,
        }),
    )?;
    sessions::finalize(&conn, session_id);
    Ok(())
}

fn reimport_source(state: &AppState, payload: &Value) -> Result<(), String> {
    let source_id = source_id_of(payload)?;
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let source = sources::find_by_id(&conn, &source_id)?;
    if source.status != "active" && source.status != "error" {
        return Ok(()); // skip non-active/error sources
    }
    sources::update_source(
        state,
        &conn,
        &source_id,
        &json!({ "status": "importing", "lastImportStartedAt": true }),
    )?;
    let import_sources = list_import_sources(&source)?;
    if import_sources.is_empty() {
        sources::update_source(
            state,
            &conn,
            &source_id,
            &json!({
                "status": "active",
                "lastImportFinishedAt": true,
                "lastImportStatusMessage": "Re-import complete. No import sources found.",
            }),
        )?;
        return Ok(());
    }
    let session_id = sessions::create(
        &conn,
        &source_id,
        import_sources.len() as i64,
        false,
        total_input_bytes(&source),
    )?;
    for import_source in import_sources {
        queue::send_job(
            state,
            "ingestion",
            "process-mailbox",
            &json!({
                "ingestionSourceId": source.id,
                "importSource": import_source,
                "sessionId": session_id,
            }),
            queue::no_retry(),
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
