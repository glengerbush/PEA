//! Write endpoints — ports of the Node controllers' POST/PUT/PATCH/DELETE
//! surface, byte-matching response shapes and error messages.

use crate::state::AppState;
use crate::{emails, queue, sources};
use axum::extract::{Multipart, Path as AxumPath, State};
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
// POST /upload — multipart file → open-archiver/tmp/<uuid>-<name>
// ---------------------------------------------------------------------------

pub async fn upload_file(State(app): State<AppState>, mut multipart: Multipart) -> Response {
    let mut file_path = String::new();
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let Some(_name) = field.name().map(String::from) else { continue };
                let original = field.file_name().unwrap_or("file").to_string();
                let bytes = match field.bytes().await {
                    Ok(b) => b,
                    Err(_) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "status": "error",
                                "statusCode": 500,
                                "message": "Error processing the upload stream.",
                                "errors": Value::Null,
                            })),
                        )
                            .into_response();
                    }
                };
                file_path = format!(
                    "open-archiver/tmp/{}-{}",
                    uuid::Uuid::new_v4(),
                    sanitize_upload_filename(&original)
                );
                if app.storage_put(&file_path, &bytes).is_err() {
                    return internal_error();
                }
            }
            Ok(None) => break,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "status": "error",
                        "statusCode": 400,
                        "message": "Invalid upload request.",
                        "errors": Value::Null,
                    })),
                )
                    .into_response();
            }
        }
    }
    Json(json!({ "filePath": file_path })).into_response()
}

fn sanitize_upload_filename(name: &str) -> String {
    let base = name.trim().rsplit(['/', '\\']).next().unwrap_or("").to_string();
    let cleaned = regex::Regex::new(r"[\\/ ]+")
        .unwrap()
        .replace_all(&base, "_")
        .trim()
        .to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "file".into()
    } else {
        cleaned
    }
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

pub async fn trigger_import(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    if sources::safe_source_json(&conn, &id).is_none() {
        return not_found("Ingestion source not found");
    }
    sources::trigger_initial_import(&app, &id);
    message_response(StatusCode::ACCEPTED, "Initial import triggered successfully.")
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

pub async fn force_sync(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    match sources::trigger_force_sync(&app, &conn, &id) {
        Ok(()) => message_response(StatusCode::ACCEPTED, "Force sync triggered successfully."),
        Err(e) if e == "Ingestion source not found" => not_found("Ingestion source not found"),
        Err(_) => internal_error(),
    }
}

pub async fn unmerge_source(State(app): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = app.pool.get().unwrap();
    let source = match sources::find_by_id(&app, &conn, &id) {
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
    match emails::delete_archived_email(&app, &conn, &id) {
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
        match emails::delete_archived_email(&app, &conn, id) {
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
        for id in &duplicate_ids {
            if emails::delete_archived_email(&app, &conn, id).is_ok() {
                deleted_emails += 1;
            }
        }
        if keeper_exists {
            keeper_emails += 1;
        }
        approved_groups += 1;
    }
    Json(json!({
        "approvedGroups": approved_groups,
        "deletedEmails": deleted_emails,
        "keeperEmails": keeper_emails,
    }))
    .into_response()
}

pub async fn scan_fuzzy(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let batch_size = dto
        .get("batchSize")
        .and_then(|v| v.as_i64())
        .filter(|n| *n >= 1)
        .map(|n| n.min(500))
        .unwrap_or(100);
    let job_id = queue::send_job(
        &app,
        "indexing",
        "scan-fuzzy-duplicates",
        &json!({ "batchSize": batch_size }),
        queue::SendOptions::default(),
    );
    (
        StatusCode::ACCEPTED,
        Json(json!({ "jobId": job_id.unwrap_or_default(), "batchSize": batch_size })),
    )
        .into_response()
}

pub async fn approve_fuzzy(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let groups = dto.get("groups").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut approved_groups = 0usize;
    let mut deleted_emails = 0usize;
    let mut keeper_emails = 0usize;
    for group in &groups {
        let group_id = group.get("groupId").and_then(|v| v.as_str()).unwrap_or("");
        let keeper = group.get("keeperEmailId").and_then(|v| v.as_str()).unwrap_or("");
        let mut duplicate_ids: Vec<String> = Vec::new();
        for v in group.get("duplicateEmailIds").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
            if let Some(s) = v.as_str() {
                if s != keeper && !duplicate_ids.contains(&s.to_string()) {
                    duplicate_ids.push(s.to_string());
                }
            }
        }
        if group_id.is_empty() || keeper.is_empty() || duplicate_ids.is_empty() {
            continue;
        }
        let keeper_exists: bool = conn
            .query_row("SELECT 1 FROM archived_emails WHERE id = ?", [keeper], |_| Ok(true))
            .unwrap_or(false);
        for id in &duplicate_ids {
            if emails::delete_archived_email(&app, &conn, id).is_ok() {
                deleted_emails += 1;
            }
        }
        conn.execute(
            "UPDATE fuzzy_duplicate_groups SET status = 'approved', updated_at = ? WHERE id = ?",
            rusqlite::params![crate::search::now_ms(), group_id],
        )
        .ok();
        if keeper_exists {
            keeper_emails += 1;
        }
        approved_groups += 1;
    }
    Json(json!({
        "approvedGroups": approved_groups,
        "deletedEmails": deleted_emails,
        "keeperEmails": keeper_emails,
    }))
    .into_response()
}

pub async fn ignore_fuzzy(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let mut ids: Vec<String> = Vec::new();
    for v in dto.get("groupIds").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
        if let Some(s) = v.as_str() {
            if !s.is_empty() && !ids.contains(&s.to_string()) {
                ids.push(s.to_string());
            }
        }
    }
    if ids.is_empty() {
        return Json(json!({ "ignoredGroups": 0 })).into_response();
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut params: Vec<rusqlite::types::Value> =
        vec![rusqlite::types::Value::from(crate::search::now_ms())];
    params.extend(ids.iter().map(|s| rusqlite::types::Value::from(s.clone())));
    let changed = conn
        .execute(
            &format!(
                "UPDATE fuzzy_duplicate_groups SET status = 'ignored', updated_at = ? WHERE id IN ({placeholders})"
            ),
            rusqlite::params_from_iter(params.iter()),
        )
        .unwrap_or(0);
    Json(json!({ "ignoredGroups": changed })).into_response()
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
    r#"{"language":"en","theme":"system","timeZone":null,"clockFormat":"12h"}"#;

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

pub async fn rebuild_search(State(app): State<AppState>) -> Response {
    let conn = app.pool.get().unwrap();
    if crate::provision::ensure_fts(&conn).is_err() {
        return message_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to start the search index rebuild.",
        );
    }
    conn.execute("DELETE FROM email_fts", []).ok();
    let mut stmt = conn
        .prepare("SELECT id FROM archived_emails ORDER BY archived_at ASC")
        .unwrap();
    let ids: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    const BATCH: usize = 500;
    let mut batches = 0usize;
    for chunk in ids.chunks(BATCH) {
        let emails: Vec<Value> = chunk.iter().map(|id| json!({ "archivedEmailId": id })).collect();
        queue::send_job(
            &app,
            "indexing",
            "index-email-batch",
            &json!({ "emails": emails }),
            queue::SendOptions::default(),
        );
        batches += 1;
    }
    (
        StatusCode::ACCEPTED,
        Json(json!({ "enqueuedBatches": batches, "totalEmails": ids.len() })),
    )
        .into_response()
}

pub async fn update_profile(State(app): State<AppState>, Json(dto): Json<Value>) -> Response {
    let conn = app.pool.get().unwrap();
    let user_id: Option<String> = conn
        .query_row("SELECT id FROM users ORDER BY created_at ASC LIMIT 1", [], |r| r.get(0))
        .ok();
    let Some(user_id) = user_id else { return message_response(StatusCode::UNAUTHORIZED, "Unauthorized") };
    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    for (key, col) in [("email", "email"), ("first_name", "first_name"), ("last_name", "last_name")] {
        if let Some(v) = dto.get(key) {
            sets.push(format!("{col} = ?"));
            match v.as_str() {
                Some(s) => params.push(s.to_string().into()),
                None => params.push(rusqlite::types::Value::Null),
            }
        }
    }
    if !sets.is_empty() {
        let sql = format!("UPDATE users SET {} WHERE id = ?", sets.join(", "));
        params.push(user_id.clone().into());
        if conn.execute(&sql, rusqlite::params_from_iter(params.iter())).is_err() {
            return internal_error();
        }
    }
    // toPublicUser — the same five fields as GET /users/profile.
    let user = conn
        .query_row(
            "SELECT id, email, first_name, last_name, created_at FROM users WHERE id = ?",
            [&user_id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "email": row.get::<_, String>(1)?,
                    "first_name": row.get::<_, Option<String>>(2)?,
                    "last_name": row.get::<_, Option<String>>(3)?,
                    "createdAt": crate::iso(row.get::<_, i64>(4)?),
                }))
            },
        )
        .unwrap_or(Value::Null);
    Json(user).into_response()
}

/// GET /settings/updates/check — the legacy commit-based GitHub check.
pub async fn check_updates(State(_app): State<AppState>) -> Response {
    let git_sha = std::env::var("PEA_GIT_SHA")
        .or_else(|_| std::env::var("OA_GIT_SHA"))
        .unwrap_or_else(|_| "unknown".into());
    let repo = std::env::var("PEA_UPDATE_REPO")
        .or_else(|_| std::env::var("OA_UPDATE_REPO"))
        .unwrap_or_else(|_| "glengerbush/PEA".into());
    let branch = std::env::var("PEA_UPDATE_BRANCH")
        .or_else(|_| std::env::var("OA_UPDATE_BRANCH"))
        .unwrap_or_else(|_| "main".into());
    let update_command = std::env::var("PEA_UPDATE_COMMAND")
        .or_else(|_| std::env::var("OA_UPDATE_COMMAND"))
        .unwrap_or_default();

    let checked_at = crate::iso(crate::search::now_ms());
    let mut base = json!({
        "currentSha": git_sha,
        "latestSha": Value::Null,
        "updateAvailable": false,
        "behindBy": 0,
        "commits": [],
        "compareUrl": Value::Null,
        "checkedAt": checked_at,
        "updateCommand": update_command,
    });

    let client = reqwest::Client::builder()
        .user_agent("PEA-UpdateCheck")
        .build();
    let Ok(client) = client else {
        return message_response(StatusCode::BAD_GATEWAY, "Failed to reach GitHub to check for updates.");
    };
    let branch_res = client
        .get(format!("https://api.github.com/repos/{repo}/commits/{branch}"))
        .header("accept", "application/vnd.github+json")
        .send()
        .await;
    let branch_res = match branch_res {
        Ok(r) => r,
        Err(_) => {
            return message_response(
                StatusCode::BAD_GATEWAY,
                "Failed to reach GitHub to check for updates.",
            )
        }
    };
    if !branch_res.status().is_success() {
        let status = branch_res.status().as_u16();
        base["status"] = json!("error");
        base["message"] = json!(format!(
            "GitHub API returned {status} while resolving {repo}@{branch}."
        ));
        return Json(base).into_response();
    }
    let latest_sha = branch_res
        .json::<Value>()
        .await
        .ok()
        .and_then(|v| v.get("sha").and_then(|s| s.as_str()).map(String::from))
        .unwrap_or_default();
    base["latestSha"] = json!(latest_sha);

    if git_sha.is_empty() || git_sha == "unknown" {
        base["status"] = json!("unknown");
        base["message"] = json!(
            "This build wasn't stamped with a commit, so update status can't be determined."
        );
        return Json(base).into_response();
    }
    if git_sha == latest_sha {
        base["status"] = json!("up_to_date");
        return Json(base).into_response();
    }
    let cmp_res = client
        .get(format!(
            "https://api.github.com/repos/{repo}/compare/{git_sha}...{latest_sha}"
        ))
        .header("accept", "application/vnd.github+json")
        .send()
        .await;
    let cmp: Option<Value> = match cmp_res {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        _ => None,
    };
    let Some(cmp) = cmp else {
        base["status"] = json!("unknown");
        base["message"] = json!(
            "The deployed commit isn't on the remote, so how far behind can't be computed."
        );
        return Json(base).into_response();
    };
    let cmp_status = cmp.get("status").and_then(|s| s.as_str()).unwrap_or("");
    let behind_by = if cmp_status == "ahead" || cmp_status == "diverged" {
        cmp.get("ahead_by").and_then(|n| n.as_i64()).unwrap_or(0)
    } else {
        0
    };
    let commits: Vec<Value> = cmp
        .get("commits")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    json!({
                        "sha": c.get("sha").cloned().unwrap_or(Value::Null),
                        "message": c
                            .get("commit")
                            .and_then(|x| x.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .split('\n')
                            .next()
                            .unwrap_or(""),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    base["behindBy"] = json!(behind_by);
    base["commits"] = json!(commits);
    base["compareUrl"] = cmp.get("html_url").cloned().unwrap_or(Value::Null);
    base["updateAvailable"] = json!(behind_by > 0);
    base["status"] = json!(if behind_by > 0 { "update_available" } else { "up_to_date" });
    Json(base).into_response()
}

