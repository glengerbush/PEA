//! Port of IngestionService's source lifecycle: create (derive name, encrypt
//! credentials, test connection, auto-trigger import), update (re-encrypt,
//! auth_success → initial import), delete (children, storage, FTS, row),
//! unmerge, force sync, and credential decryption.

use crate::state::AppState;
use crate::{crypto, queue};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Value};

pub const FILE_BASED_PROVIDERS: [&str; 3] = ["eml_import", "mbox_import", "pst_import"];

fn now_ms() -> i64 {
    crate::search::now_ms()
}

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

    static MBOX_SEG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)([^/]+)\.mbox(?:/|$)").unwrap());
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
        static EXT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\.(mbox|eml|emlx)$").unwrap());
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
    /// Decrypted providerConfig plus {"type": provider} (decryptSource shape).
    pub credentials: Value,
}

pub fn find_by_id(state: &AppState, conn: &Connection, id: &str) -> Result<SourceRow, String> {
    let row = conn
        .query_row(
            "SELECT id, name, provider, status, merged_into_id, credentials \
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
    let mut credentials = row
        .5
        .as_deref()
        .and_then(|enc| {
            let master = state.master_key.as_deref()?;
            crypto::decrypt_credentials(enc, master)
        })
        .and_then(|json| serde_json::from_str::<Value>(&json).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = credentials.as_object_mut() {
        obj.insert("type".into(), json!(row.2.clone()));
    }
    Ok(SourceRow {
        id: row.0,
        name: row.1,
        provider: row.2,
        status: row.3,
        merged_into_id: row.4,
        credentials,
    })
}

/// The toSafeIngestionSource JSON (no credentials) for a source id.
pub fn safe_source_json(conn: &Connection, id: &str) -> Option<Value> {
    conn.query_row(
        "SELECT id, user_id, name, provider, status, last_sync_started_at, \
         last_sync_finished_at, last_sync_status_message, sync_state, merged_into_id, \
         created_at, updated_at FROM ingestion_sources WHERE id = ?",
        [id],
        |row| {
            let sync_state: Option<String> = row.get(8)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "userId": row.get::<_, Option<String>>(1)?,
                "name": row.get::<_, String>(2)?,
                "provider": row.get::<_, String>(3)?,
                "status": row.get::<_, String>(4)?,
                "lastSyncStartedAt": row.get::<_, Option<i64>>(5)?.map(crate::iso),
                "lastSyncFinishedAt": row.get::<_, Option<i64>>(6)?.map(crate::iso),
                "lastSyncStatusMessage": row.get::<_, Option<String>>(7)?,
                "syncState": sync_state.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
                "mergedIntoId": row.get::<_, Option<String>>(9)?,
                "createdAt": crate::iso(row.get::<_, i64>(10)?),
                "updatedAt": crate::iso(row.get::<_, i64>(11)?),
            }))
        },
    )
    .ok()
}

/// Applies an update dto (status/name/lastSync* / providerConfig) and fires the
/// initial import when status transitions to auth_success. Mirrors update().
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
    if let Some(msg) = dto.get("lastSyncStatusMessage").and_then(|v| v.as_str()) {
        push(&mut sets, &mut params, "last_sync_status_message", msg.to_string().into());
    }
    if dto.get("lastSyncFinishedAt").is_some() {
        push(&mut sets, &mut params, "last_sync_finished_at", now_ms().into());
    }
    if dto.get("lastSyncStartedAt").is_some() {
        push(&mut sets, &mut params, "last_sync_started_at", now_ms().into());
    }
    if let Some(config) = dto.get("providerConfig") {
        let master = state.master_key.as_deref().ok_or("no master key")?;
        let enc = crypto::encrypt_credentials(&config.to_string(), master);
        push(&mut sets, &mut params, "credentials", enc.into());
    }
    if !sets.is_empty() {
        // drizzle .set() doesn't touch updated_at implicitly, so neither do we.
        let sql = format!("UPDATE ingestion_sources SET {} WHERE id = ?", sets.join(", "));
        params.push(id.to_string().into());
        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
            .map_err(|e| e.to_string())?;
    }

    let new_status = dto.get("status").and_then(|v| v.as_str());
    if original_status != "auth_success" && new_status == Some("auth_success") {
        trigger_initial_import(state, id);
    }
    Ok(())
}

/// IngestionService.create — derive name, encrypt credentials, insert
/// pending_auth, test the connection, then flip to auth_success (which
/// triggers the initial import). On test failure the source is deleted and
/// the error message is surfaced (the endpoint returns it as a 400).
pub fn create_source(state: &AppState, conn: &Connection, dto: &Value) -> Result<String, String> {
    let provider = dto
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or("provider is required")?
        .to_string();
    let provider_config = dto.get("providerConfig").cloned().unwrap_or_else(|| json!({}));
    let master = state.master_key.as_deref().ok_or("no master key")?;
    let credentials = crypto::encrypt_credentials(&provider_config.to_string(), master);

    // Resolve merge target: a child target is followed to its root.
    let merged_into_id: Option<String> = match dto.get("mergedIntoId").and_then(|v| v.as_str()) {
        Some(target_id) => {
            let target = find_by_id(state, conn, target_id)?;
            Some(target.merged_into_id.unwrap_or(target.id))
        }
        None => None,
    };
    let name = derive_source_name(dto);
    let user_id = crate::provision::ensure_local_user(conn)?;
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO ingestion_sources (id, user_id, name, provider, credentials, status, merged_into_id) \
         VALUES (?, ?, ?, ?, ?, 'pending_auth', ?)",
        rusqlite::params![id, user_id, name, provider, credentials, merged_into_id],
    )
    .map_err(|e| e.to_string())?;

    let source = find_by_id(state, conn, &id)?;
    let test = match provider.as_str() {
        "mbox_import" => crate::connectors::get_mbox_inputs(state, &source.credentials).map(|_| ()),
        "eml_import" => crate::eml::validate(state, &source.credentials),
        other => Err(format!("Unsupported provider: {other}")),
    };
    match test {
        Ok(()) => {
            update_source(state, conn, &id, &json!({ "status": "auth_success" }))?;
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

pub fn trigger_force_sync(state: &AppState, conn: &Connection, id: &str) -> Result<(), String> {
    let source = find_by_id(state, conn, id)?;
    queue::remove_jobs_by_source_id(conn, id);
    update_source(
        state,
        conn,
        id,
        &json!({ "status": "active", "lastSyncStatusMessage": "Force sync triggered by user." }),
    )?;
    queue::send_job(
        state,
        "ingestion",
        "continuous-sync",
        &json!({ "ingestionSourceId": source.id }),
        queue::master_job_options(&format!("continuous-sync:{}", source.id)),
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
                    "continuous-sync",
                    &json!({ "ingestionSourceId": child_id }),
                    queue::master_job_options(&format!("continuous-sync:{child_id}")),
                );
            }
        }
    }
    Ok(())
}

/// delete() — children first, then storage prefix, uploaded files, FTS, row.
pub fn delete_source(state: &AppState, conn: &Connection, id: &str) -> Result<(), String> {
    let source = find_by_id(state, conn, id)?;

    if source.merged_into_id.is_none() {
        let mut stmt = conn
            .prepare("SELECT id FROM ingestion_sources WHERE merged_into_id = ?")
            .map_err(|e| e.to_string())?;
        let children: Vec<String> = stmt
            .query_map([id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        for child in children {
            delete_source(state, conn, &child)?;
        }
    }

    // Storage prefix for this source's emails + attachments.
    let prefix = format!("open-archiver/{}-{}/", source.name.replace(' ', "-"), source.id);
    let dir = state.storage_root().join(&prefix);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }

    // Leftover uploaded files (eml/mbox imports).
    if source.provider == "eml_import" || source.provider == "mbox_import" {
        let mut uploaded: Vec<String> = Vec::new();
        if let Some(p) = source.credentials.get("uploadedFilePath").and_then(|v| v.as_str()) {
            uploaded.push(p.to_string());
        }
        if source.provider == "mbox_import" {
            if let Some(files) = source.credentials.get("uploadedFiles").and_then(|v| v.as_array()) {
                for f in files {
                    if let Some(p) = f.get("filePath").and_then(|v| v.as_str()) {
                        uploaded.push(p.to_string());
                    }
                }
            }
        }
        for path in uploaded {
            let file = state.storage_root().join(&path);
            if file.exists() {
                std::fs::remove_file(&file).ok();
            }
        }
    }

    // FTS rows must go while archived_emails rows still exist (rowid mapping).
    conn.execute(
        "DELETE FROM email_fts WHERE rowid IN \
         (SELECT rowid FROM archived_emails WHERE ingestion_source_id = ?)",
        [id],
    )
    .ok();

    conn.execute("DELETE FROM ingestion_sources WHERE id = ?", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
