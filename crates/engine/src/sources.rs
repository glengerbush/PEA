//! Port of IngestionService's source lifecycle: create (derive name, test
//! connection, auto-trigger import), update (ready → initial import),
//! delete (children, storage, FTS, row), unmerge, and re-import.

use crate::state::AppState;
use crate::queue;
use std::sync::LazyLock;
use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Value};

pub const FILE_BASED_PROVIDERS: [&str; 2] = ["eml_import", "mbox_import"];

/// deriveSourceName — mbox folder → top-level folder → single filename →
/// "<provider minus _import> import".
pub fn derive_source_name(dto: &Value) -> String {
    let explicit = dto.get("name").and_then(|n| n.as_str()).unwrap_or("").trim();
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    let config = dto.get("providerConfig");
    let files: Vec<&Value> = config
        .and_then(|c| c.get("uploadedFiles"))
        .and_then(|f| f.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    static MBOX_SEG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)([^/]+)\.mbox(?:/|$)").unwrap());
    for file in &files {
        let rel = file.get("relativePath").and_then(|r| r.as_str()).unwrap_or("");
        if let Some(m) = MBOX_SEG.captures(rel) {
            return m[1].to_string();
        }
    }
    let top_folder = files
        .iter()
        .map(|f| {
            f.get("relativePath")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .split('/')
                .next()
                .unwrap_or("")
        })
        .find(|part| !part.is_empty() && !part.contains('.'));
    if let Some(folder) = top_folder {
        return folder.to_string();
    }
    let single = config
        .and_then(|c| c.get("uploadedFileName"))
        .and_then(|n| n.as_str())
        .map(String::from)
        .or_else(|| {
            files
                .first()
                .and_then(|f| f.get("fileName"))
                .and_then(|n| n.as_str())
                .map(String::from)
        });
    if let Some(name) = single {
        static EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(mbox|eml|emlx)$").unwrap());
        return EXT.replace(&name, "").to_string();
    }
    let provider = dto.get("provider").and_then(|p| p.as_str()).unwrap_or("import");
    format!("{} import", provider.trim_end_matches("_import"))
}

pub struct SourceRow {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub status: String,
    pub merged_into_id: Option<String>,
    /// The stored providerConfig plus {"type": provider}. Plain JSON (file
    /// paths for the mbox/eml importers) — nothing is decrypted; not a secret.
    pub provider_config: Value,
}

pub fn find_by_id(conn: &Connection, id: &str) -> Result<SourceRow, String> {
    let row = conn
        .query_row(
            "SELECT id, name, provider, status, merged_into_id, provider_config \
             FROM ingestion_sources WHERE id = ?",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|_| "Ingestion source not found".to_string())?;
    // Provider config is plain JSON — file paths, nothing secret for a local
    // single-user archive.
    let mut provider_config = row
        .5
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = provider_config.as_object_mut() {
        obj.insert("type".into(), json!(row.2.clone()));
    }
    Ok(SourceRow {
        id: row.0,
        name: row.1,
        provider: row.2,
        status: row.3,
        merged_into_id: row.4,
        provider_config,
    })
}

/// The toSafeIngestionSource JSON (provider config omitted) for a source id.
pub fn safe_source_json(conn: &Connection, id: &str) -> Option<Value> {
    conn.query_row(
        "SELECT id, name, provider, status, last_import_started_at, \
         last_import_finished_at, last_import_status_message, merged_into_id, \
         created_at, updated_at FROM ingestion_sources WHERE id = ?",
        [id],
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "lastImportStartedAt": row.get::<_, Option<i64>>(4)?.map(crate::iso),
                "lastImportFinishedAt": row.get::<_, Option<i64>>(5)?.map(crate::iso),
                "lastImportStatusMessage": row.get::<_, Option<String>>(6)?,
                "mergedIntoId": row.get::<_, Option<String>>(7)?,
                "createdAt": crate::iso(row.get::<_, i64>(8)?),
                "updatedAt": crate::iso(row.get::<_, i64>(9)?),
            }))
        },
    )
    .ok()
}

/// Applies an update dto (status/name/lastImport* / providerConfig) and fires the
/// initial import when status transitions to ready. Mirrors update().
pub fn update_source(
    state: &AppState,
    conn: &Connection,
    id: &str,
    dto: &Value,
) -> Result<(), String> {
    let original_status: String = conn
        .query_row("SELECT status FROM ingestion_sources WHERE id = ?", [id], |r| r.get(0))
        .map_err(|_| "Ingestion source not found".to_string())?;

    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    let push = |sets: &mut Vec<String>, params: &mut Vec<rusqlite::types::Value>, col: &str, v: rusqlite::types::Value| {
        sets.push(format!("{col} = ?"));
        params.push(v);
    };
    if let Some(name) = dto.get("name").and_then(|v| v.as_str()) {
        push(&mut sets, &mut params, "name", name.to_string().into());
    }
    if let Some(status) = dto.get("status").and_then(|v| v.as_str()) {
        push(&mut sets, &mut params, "status", status.to_string().into());
    }
    if let Some(msg) = dto.get("lastImportStatusMessage").and_then(|v| v.as_str()) {
        push(&mut sets, &mut params, "last_import_status_message", msg.to_string().into());
    }
    // Persist the PROVIDED lastImport* value (epoch-ms number or ISO string; an
    // explicit null clears the column) rather than overwriting it with now.
    let ts_value = |v: &Value| -> Option<rusqlite::types::Value> {
        if v.is_null() {
            return Some(rusqlite::types::Value::Null);
        }
        if let Some(n) = v.as_i64() {
            return Some(n.into());
        }
        v.as_str().and_then(crate::search::parse_timestamp).map(Into::into)
    };
    if let Some(val) = dto.get("lastImportFinishedAt").and_then(ts_value) {
        push(&mut sets, &mut params, "last_import_finished_at", val);
    }
    if let Some(val) = dto.get("lastImportStartedAt").and_then(ts_value) {
        push(&mut sets, &mut params, "last_import_started_at", val);
    }
    if let Some(config) = dto.get("providerConfig") {
        // Stored as plain JSON — see the note on find_by_id.
        push(&mut sets, &mut params, "provider_config", config.to_string().into());
    }
    if !sets.is_empty() {
        // drizzle .set() doesn't touch updated_at implicitly, so neither do we.
        let sql = format!("UPDATE ingestion_sources SET {} WHERE id = ?", sets.join(", "));
        params.push(id.to_string().into());
        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
            .map_err(|e| e.to_string())?;
    }

    let new_status = dto.get("status").and_then(|v| v.as_str());
    if original_status != "ready" && new_status == Some("ready") {
        trigger_initial_import(state, id);
    }
    Ok(())
}

/// IngestionService.create — derive name, store provider config as plain JSON, insert
/// pending, test the connection, then flip to ready (which
/// triggers the initial import). On test failure the source is deleted and
/// the error message is surfaced (the endpoint returns it as a 400).
pub fn create_source(state: &AppState, conn: &Connection, dto: &Value) -> Result<String, String> {
    let provider = dto
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or("provider is required")?
        .to_string();
    let provider_config = dto.get("providerConfig").cloned().unwrap_or_else(|| json!({}));
    // Stored as plain JSON — see the note on find_by_id.
    let provider_config_json = provider_config.to_string();

    // Resolve merge target: a child target is followed to its root.
    let merged_into_id: Option<String> = match dto.get("mergedIntoId").and_then(|v| v.as_str()) {
        Some(target_id) => {
            let target = find_by_id(conn, target_id)?;
            Some(target.merged_into_id.unwrap_or(target.id))
        }
        None => None,
    };
    let name = derive_source_name(dto);
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO ingestion_sources (id, name, provider, provider_config, status, merged_into_id) \
         VALUES (?, ?, ?, ?, 'pending', ?)",
        rusqlite::params![id, name, provider, provider_config_json, merged_into_id],
    )
    .map_err(|e| e.to_string())?;

    let source = find_by_id(conn, &id)?;
    let test = match provider.as_str() {
        "mbox_import" => crate::readers::get_mbox_inputs(&source.provider_config).map(|_| ()),
        "eml_import" => crate::eml::validate(&source.provider_config),
        other => Err(format!("Unsupported provider: {other}")),
    };
    match test {
        Ok(()) => {
            update_source(state, conn, &id, &json!({ "status": "ready" }))?;
            Ok(id)
        }
        Err(e) => {
            delete_source(state, conn, &id).ok();
            Err(e)
        }
    }
}

pub fn trigger_initial_import(state: &AppState, id: &str) {
    queue::send_job(
        state,
        "ingestion",
        "initial-import",
        &json!({ "ingestionSourceId": id }),
        queue::master_job_options(&format!("initial-import:{id}")),
    );
}

pub fn trigger_reimport(state: &AppState, conn: &Connection, id: &str) -> Result<(), String> {
    let source = find_by_id(conn, id)?;
    queue::remove_jobs_by_source_id(conn, id);
    update_source(
        state,
        conn,
        id,
        &json!({ "status": "active", "lastImportStatusMessage": "Re-import triggered by user." }),
    )?;
    queue::send_job(
        state,
        "ingestion",
        "reimport",
        &json!({ "ingestionSourceId": source.id }),
        queue::master_job_options(&format!("reimport:{}", source.id)),
    );
    if source.merged_into_id.is_none() {
        let mut stmt = conn
            .prepare("SELECT id, provider, status FROM ingestion_sources WHERE merged_into_id = ?")
            .map_err(|e| e.to_string())?;
        let children: Vec<(String, String, String)> = stmt
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        for (child_id, provider, status) in children {
            if !FILE_BASED_PROVIDERS.contains(&provider.as_str())
                && (status == "active" || status == "error")
            {
                queue::send_job(
                    state,
                    "ingestion",
                    "reimport",
                    &json!({ "ingestionSourceId": child_id }),
                    queue::master_job_options(&format!("reimport:{child_id}")),
                );
            }
        }
    }
    Ok(())
}

/// delete() — children first, then a FK-safe DB teardown, then on-disk blobs.
///
/// The DB deletes run in dependency order *before* any file/FTS removal: a
/// single `DELETE FROM ingestion_sources` cannot be relied upon, because
/// `email_attachments.attachment_id` is `ON DELETE restrict` and the cascade
/// can try to remove an `attachments` row before the referencing junction row
/// has been cascaded away, aborting the whole statement. If that happened
/// after we'd already wiped the storage files and FTS rows, the archive would
/// be left corrupted (emails listed but bodies gone). Doing the DB work first,
/// and only removing files once it has succeeded, keeps delete atomic-enough.
pub fn delete_source(state: &AppState, conn: &Connection, id: &str) -> Result<(), String> {
    // Verify existence up front (this is the error the endpoint maps to 404) so a
    // missing id never opens an empty transaction.
    find_by_id(conn, id)?;

    // One transaction spanning this source AND all its merged children: a
    // mid-teardown failure (SQLITE_BUSY, I/O error) now rolls the whole thing back
    // instead of leaving orphaned junction rows or half-deleted children. Files
    // are removed only after the DB work commits, so the archive is never left
    // with emails listed but their bytes gone.
    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<(String, String)> = Vec::new();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    delete_source_rows(conn, id, &mut files, &mut dirs)?;
    tx.commit().map_err(|e| e.to_string())?;

    // The DB is now consistent; remove the blobs it used to reference.
    for path in files {
        if let Ok(file) = state.storage_abs(&path) {
            std::fs::remove_file(file).ok();
        }
    }
    // Drop each (now-empty) source directory if its name still matches on disk.
    for (name, sid) in dirs {
        let dir = state
            .storage_root()
            .join(format!("pea/{}-{}/", name.replace(' ', "-"), sid));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).ok();
        }
    }
    Ok(())
}

/// Recursive DB-only teardown for a source and its merged children, run inside
/// the caller's transaction. Collects the on-disk paths (blob files + source
/// directories) to remove after that transaction commits.
fn delete_source_rows(
    conn: &Connection,
    id: &str,
    files: &mut Vec<String>,
    dirs: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let source = find_by_id(conn, id)?;

    if source.merged_into_id.is_none() {
        let children: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM ingestion_sources WHERE merged_into_id = ?")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([id], |row| row.get(0))
                .map_err(|e| e.to_string())?;
            rows.filter_map(Result::ok).collect()
        };
        for child in children {
            delete_source_rows(conn, &child, files, dirs)?;
        }
    }

    // Collect the actual on-disk paths from the DB before deleting the rows, so
    // a source renamed after ingest (which never moves files) can't orphan its
    // blobs — we delete exactly what the DB references, not a name-derived guess.
    let mut collect = |sql: &str| -> Result<(), String> {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for path in rows.filter_map(Result::ok) {
            files.push(path);
        }
        Ok(())
    };
    collect("SELECT storage_path FROM archived_emails WHERE ingestion_source_id = ?")?;
    collect("SELECT storage_path FROM attachments WHERE ingestion_source_id = ?")?;
    collect(
        "SELECT storage_path FROM remote_content_assets WHERE storage_path IS NOT NULL \
         AND email_id IN (SELECT id FROM archived_emails WHERE ingestion_source_id = ?)",
    )?;

    // DB teardown in FK-safe order. Junction rows before the attachments they
    // RESTRICT; FTS before archived_emails (its rowid subquery needs the rows);
    // archived_emails cascades remote_content_assets. The likely-duplicate ignore ledger is keyed by group key, not email id.
    conn.execute(
        "DELETE FROM email_attachments WHERE email_id IN \
         (SELECT id FROM archived_emails WHERE ingestion_source_id = ?)",
        [id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM attachments WHERE ingestion_source_id = ?", [id])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM email_fts WHERE rowid IN \
         (SELECT rowid FROM archived_emails WHERE ingestion_source_id = ?)",
        [id],
    )
    .ok();
    conn.execute("DELETE FROM archived_emails WHERE ingestion_source_id = ?", [id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM ingestion_sources WHERE id = ?", [id])
        .map_err(|e| e.to_string())?;

    dirs.push((source.name, source.id));
    Ok(())
}
