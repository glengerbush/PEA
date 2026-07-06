//! Write endpoints: POST/PUT/PATCH/DELETE handlers for the HTTP API.

use crate::state::AppState;
use crate::{emails, sources};
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "message": "An internal server error occurred" })),
    )
        .into_response()
}

fn not_found(message: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "message": message }))).into_response()
}

fn message_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

// ---------------------------------------------------------------------------
// Ingestion sources
// ---------------------------------------------------------------------------

pub async fn create_source(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    match sources::create_source(&app, &conn, &dto) {
        Ok(id) => match sources::safe_source_json(&conn, &id) {
            Some(source) => (StatusCode::CREATED, Json(source)).into_response(),
            None => internal_error(),
        },
        Err(message) => message_response(StatusCode::BAD_REQUEST, &message),
    }
}

pub async fn update_source(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(dto): Json<Value>,
) -> Response {
    let conn = app.pool.get().unwrap();
    match sources::update_source(&app, &conn, &id, &dto) {
        Ok(()) => match sources::safe_source_json(&conn, &id) {
            Some(source) => Json(source).into_response(),
            None => not_found("Ingestion source not found"),
        },
        Err(e) if e == "Ingestion source not found" => not_found("Ingestion source not found"),
        Err(_) => internal_error(),
    }
}

pub async fn delete_source(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    match sources::delete_source(&app, &conn, &id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e == "Ingestion source not found" => not_found("Ingestion source not found"),
        Err(message) => message_response(StatusCode::BAD_REQUEST, &message),
    }
}

pub async fn pause_source(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    match sources::update_source(&app, &conn, &id, &json!({ "status": "paused" })) {
        Ok(()) => match sources::safe_source_json(&conn, &id) {
            Some(source) => Json(source).into_response(),
            None => not_found("Ingestion source not found"),
        },
        Err(e) if e == "Ingestion source not found" => not_found("Ingestion source not found"),
        Err(_) => internal_error(),
    }
}

pub async fn reimport(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    match sources::trigger_reimport(&app, &conn, &id) {
        Ok(()) => message_response(StatusCode::ACCEPTED, "Re-import triggered successfully."),
        Err(e) if e == "Ingestion source not found" => not_found("Ingestion source not found"),
        Err(_) => internal_error(),
    }
}

pub async fn unmerge_source(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    let source = match sources::find_by_id(&conn, &id) {
        Ok(s) => s,
        Err(_) => return not_found("Ingestion source not found"),
    };
    if source.merged_into_id.is_none() {
        return message_response(
            StatusCode::BAD_REQUEST,
            "Source is not merged into another source.",
        );
    }
    if conn
        .execute("UPDATE ingestion_sources SET merged_into_id = NULL WHERE id = ?", [&id])
        .is_err()
    {
        return internal_error();
    }
    match sources::safe_source_json(&conn, &id) {
        Some(source) => Json(source).into_response(),
        None => internal_error(),
    }
}

// ---------------------------------------------------------------------------
// Archived emails: delete / bulk delete / tags / duplicates / remote-content
// ---------------------------------------------------------------------------

pub async fn delete_email(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    // Deleting sends the email to the trash (soft delete); permanent removal
    // happens from the trash via "delete forever" / "empty trash".
    match emails::soft_delete_archived_email(&conn, &id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e == "Archived email not found" => not_found("Archived email not found"),
        Err(_) => internal_error(),
    }
}

pub async fn bulk_delete(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let Some(ids) = dto.get("emailIds").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) else {
        return message_response(StatusCode::BAD_REQUEST, "emailIds are required");
    };
    let mut deleted_ids: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for id in ids.iter().filter_map(|v| v.as_str()) {
        match emails::soft_delete_archived_email(&conn, id) {
            Ok(()) => deleted_ids.push(id.to_string()),
            Err(reason) => failed.push(json!({ "id": id, "reason": reason })),
        }
    }
    Json(json!({
        "requestedCount": ids.len(),
        "deletedCount": deleted_ids.len(),
        "deletedIds": deleted_ids,
        "failed": failed,
    }))
    .into_response()
}

/// Permanently delete a set of email ids (used by the trash), returning the
/// ids removed and any per-id failures. Uses the ref-counting hard delete, so a
/// blob shared with an email that remains (trashed or not) is preserved.
fn permanently_delete_ids<'a>(
    app: &AppState,
    conn: &rusqlite::Connection,
    ids: impl Iterator<Item = &'a str>,
) -> (Vec<String>, Vec<Value>) {
    let mut deleted_ids: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for id in ids {
        match emails::delete_archived_email(app, conn, id) {
            Ok(()) => deleted_ids.push(id.to_string()),
            Err(reason) => failed.push(json!({ "id": id, "reason": reason })),
        }
    }
    (deleted_ids, failed)
}

pub async fn restore_emails(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let Some(ids) = dto.get("emailIds").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) else {
        return message_response(StatusCode::BAD_REQUEST, "emailIds are required");
    };
    let mut restored_ids: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for id in ids.iter().filter_map(|v| v.as_str()) {
        match emails::restore_archived_email(&conn, id) {
            Ok(()) => restored_ids.push(id.to_string()),
            Err(reason) => failed.push(json!({ "id": id, "reason": reason })),
        }
    }
    Json(json!({
        "requestedCount": ids.len(),
        "restoredCount": restored_ids.len(),
        "restoredIds": restored_ids,
        "failed": failed,
    }))
    .into_response()
}

pub async fn permanent_delete_emails(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let Some(ids) = dto.get("emailIds").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) else {
        return message_response(StatusCode::BAD_REQUEST, "emailIds are required");
    };
    let (deleted_ids, failed) =
        permanently_delete_ids(&app, &conn, ids.iter().filter_map(|v| v.as_str()));
    Json(json!({
        "requestedCount": ids.len(),
        "deletedCount": deleted_ids.len(),
        "deletedIds": deleted_ids,
        "failed": failed,
    }))
    .into_response()
}

pub async fn empty_trash(State(app): State<AppState>) -> Response {
    let conn = app.pool.get().unwrap();
    let trashed_ids: Vec<String> = {
        let mut stmt = match conn.prepare("SELECT id FROM archived_emails WHERE deleted_at IS NOT NULL") {
            Ok(stmt) => stmt,
            Err(_) => return internal_error(),
        };
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>());
        match rows {
            Ok(ids) => ids,
            Err(_) => return internal_error(),
        }
    };
    let mut deleted_ids: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for id in &trashed_ids {
        match emails::delete_archived_email(&app, &conn, id) {
            Ok(()) => deleted_ids.push(id.clone()),
            // Deleting the last email of a file-based source auto-removes that
            // source, which can cascade away OTHER trashed rows still in this
            // list. Such an id is already gone — successfully emptied, not a
            // failure — so it must not be reported as "not found".
            Err(reason) if reason == "Archived email not found" => deleted_ids.push(id.clone()),
            Err(reason) => failed.push(json!({ "id": id, "reason": reason })),
        }
    }
    Json(json!({
        "deletedCount": deleted_ids.len(),
        "deletedIds": deleted_ids,
        "failed": failed,
    }))
    .into_response()
}

pub async fn update_tags(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    if !dto.get("emailIds").map_or(false, |v| v.is_array()) {
        return message_response(StatusCode::BAD_REQUEST, "emailIds are required");
    }
    match emails::update_email_tags(&conn, &dto) {
        Ok(result) => Json(result).into_response(),
        Err(message) => message_response(StatusCode::BAD_REQUEST, &message),
    }
}

pub async fn approve_exact_duplicates(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let groups = dto.get("groups").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut approved_groups = 0usize;
    let mut deleted_emails = 0usize;
    let mut keeper_emails = 0usize;
    for group in &groups {
        let keeper = group.get("keeperEmailId").and_then(|v| v.as_str()).unwrap_or("");
        let mut duplicate_ids: Vec<String> = Vec::new();
        for v in group.get("duplicateEmailIds").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
            if let Some(s) = v.as_str() {
                if s != keeper && !duplicate_ids.contains(&s.to_string()) {
                    duplicate_ids.push(s.to_string());
                }
            }
        }
        if keeper.is_empty() || duplicate_ids.is_empty() {
            continue;
        }
        let keeper_exists: bool = conn
            .query_row("SELECT 1 FROM archived_emails WHERE id = ?", [keeper], |_| Ok(true))
            .unwrap_or(false);
        // Never delete a cluster's copies when the keeper is gone — that would
        // destroy the last remaining copy. Skip the group entirely.
        if !keeper_exists {
            continue;
        }
        // Approved duplicate copies go to the trash (recoverable), consistent
        // with every other delete; empty the trash to reclaim their storage.
        for id in &duplicate_ids {
            if emails::soft_delete_archived_email(&conn, id).is_ok() {
                deleted_emails += 1;
            }
        }
        keeper_emails += 1;
        approved_groups += 1;
    }
    Json(json!({
        "approvedGroups": approved_groups,
        "deletedEmails": deleted_emails,
        "keeperEmails": keeper_emails,
    }))
    .into_response()
}

pub async fn ignore_exact(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let mut fingerprints: Vec<String> = Vec::new();
    for v in dto.get("groupKeys").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
        if let Some(s) = v.as_str() {
            // The client sends the cluster group key ("cluster:<fingerprint>");
            // persist just the fingerprint (the cluster's min email id).
            let fp = s.strip_prefix("cluster:").unwrap_or(s);
            if !fp.is_empty() && !fingerprints.iter().any(|f| f == fp) {
                fingerprints.push(fp.to_string());
            }
        }
    }
    if fingerprints.is_empty() {
        return Json(json!({ "ignoredGroups": 0 })).into_response();
    }
    // Record each ignored fingerprint; list_exact_groups excludes these clusters.
    let mut ignored = 0usize;
    for fp in &fingerprints {
        if conn
            .execute(
                "INSERT OR IGNORE INTO exact_duplicate_ignores (fingerprint) VALUES (?)",
                [fp],
            )
            .unwrap_or(0)
            > 0
        {
            ignored += 1;
        }
    }
    Json(json!({ "ignoredGroups": ignored })).into_response()
}

pub async fn enqueue_remote_content(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let job_id = crate::remote_content::enqueue_archive(&app, &[id.clone()]);
    (
        StatusCode::ACCEPTED,
        Json(json!({ "jobId": job_id.unwrap_or_default(), "emailIds": [id] })),
    )
        .into_response()
}

/// POST /archived-emails/{id}/remote-assets/{assetId}/retry — re-fetch just this
/// one failed/blocked asset, leaving every other asset (and all already-archived
/// ones) untouched. Runs the blocking HTTP fetch off the async executor.
pub async fn retry_remote_asset(
    State(app): State<AppState>,
    AxumPath((id, asset_id)): AxumPath<(String, String)>,
) -> Response {
    let original_url: Option<String> = {
        let conn = app.pool.get().unwrap();
        conn.query_row(
            "SELECT original_url FROM remote_content_assets WHERE id = ? AND email_id = ?",
            rusqlite::params![asset_id, id],
            |r| r.get(0),
        )
        .ok()
    };
    let Some(original_url) = original_url else {
        return not_found("Remote content asset not found");
    };
    let app2 = app.clone();
    let id2 = id.clone();
    if tokio::task::spawn_blocking(move || {
        crate::remote_content::retry_asset(&app2, &id2, &original_url);
    })
    .await
    .is_err()
    {
        return internal_error();
    }
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// Contacts / settings / users / search rebuild / updates
// ---------------------------------------------------------------------------

pub async fn import_contacts(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let format = dto.get("format").and_then(|v| v.as_str()).unwrap_or("");
    let content = dto.get("content").and_then(|v| v.as_str());
    if (format != "csv" && format != "vcf") || content.is_none() {
        return message_response(
            StatusCode::BAD_REQUEST,
            "A \"format\" (csv|vcf) and \"content\" are required",
        );
    }
    let conn = app.pool.get().unwrap();
    match emails::import_contacts(&conn, format, content.unwrap()) {
        Ok(result) => Json(result).into_response(),
        Err(message) => message_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
    }
}

const DEFAULT_SETTINGS: &str =
    r#"{"language":"en","theme":"system","timeZone":null,"clockFormat":"12h","dateFormat":"system","autoCheckUpdates":true}"#;

pub async fn update_settings(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    // getSystemSettings: defaults ← stored config; create the row if missing.
    let stored: Option<(i64, String)> = conn
        .query_row("SELECT id, config FROM system_settings LIMIT 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
    let mut merged: Value = serde_json::from_str(DEFAULT_SETTINGS).unwrap();
    let (row_id, current): (i64, Value) = match stored {
        Some((id, config)) => (id, serde_json::from_str(&config).unwrap_or(json!({}))),
        None => {
            // id is INTEGER AUTOINCREMENT — let SQLite assign it.
            if conn
                .execute(
                    "INSERT INTO system_settings (config) VALUES (?)",
                    rusqlite::params![DEFAULT_SETTINGS],
                )
                .is_err()
            {
                return message_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to update settings.",
                );
            }
            (conn.last_insert_rowid(), serde_json::from_str(DEFAULT_SETTINGS).unwrap())
        }
    };
    if let (Some(m), Some(c)) = (merged.as_object_mut(), current.as_object()) {
        for (k, v) in c {
            m.insert(k.clone(), v.clone());
        }
    }
    if let (Some(m), Some(new)) = (merged.as_object_mut(), dto.as_object()) {
        for (k, v) in new {
            m.insert(k.clone(), v.clone());
        }
    }
    if conn
        .execute(
            "UPDATE system_settings SET config = ? WHERE id = ?",
            rusqlite::params![merged.to_string(), row_id],
        )
        .is_err()
    {
        return message_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to update settings.");
    }
    Json(merged).into_response()
}

