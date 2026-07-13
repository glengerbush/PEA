//! R1 read handlers beyond search: email detail / per-source listing,
//! dashboard summaries, queue details, source detail, storage download.

use crate::state::AppState;
use crate::{iso, search};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "message": "An internal server error occurred" })),
    )
        .into_response()
}

/// Flattens {to,cc,bcc} into [{name?, email?}]. `None` when the recipients JSON
/// is absent or unparseable; the caller maps that to a 500.
fn flatten_recipients(raw: Option<&str>) -> Option<Vec<Value>> {
    let parsed: Value = match raw {
        Some(s) => serde_json::from_str(s).ok()?,
        None => return None,
    };
    let obj = parsed.as_object()?;
    let mut out = Vec::new();
    for key in ["to", "cc", "bcc"] {
        let Some(list) = obj.get(key).and_then(|v| v.as_array()) else {
            continue;
        };
        for r in list {
            let mut entry = Map::new();
            // Keys appear only when present.
            if let Some(name) = r.get("name") {
                entry.insert("name".into(), name.clone());
            }
            if let Some(address) = r.get("address") {
                entry.insert("email".into(), address.clone());
            }
            // Preserve which field this recipient came from so the detail view
            // can render separate To / Cc / Bcc lines.
            entry.insert("kind".into(), json!(key));
            out.push(Value::Object(entry));
        }
    }
    Some(out)
}

/// Full archived_emails row shape, with controller-level overrides applied
/// (recipients flattened, sourceLabels/tags parsed-or-null, path empty→null).
fn email_full_row(row: &rusqlite::Row) -> rusqlite::Result<Option<Map<String, Value>>> {
    let recipients_raw: Option<String> = row.get("recipients")?;
    let Some(recipients) = flatten_recipients(recipients_raw.as_deref()) else {
        return Ok(None); // caller maps this to a 500
    };
    let mut doc = Map::new();
    doc.insert("id".into(), json!(row.get::<_, String>("id")?));
    doc.insert("threadId".into(), json!(row.get::<_, Option<String>>("thread_id")?));
    doc.insert(
        "ingestionSourceId".into(),
        json!(row.get::<_, String>("ingestion_source_id")?),
    );
    // Live ingestion-source name when the caller joined it (import_source_name),
    // else the filename-derived string frozen at ingest.
    let import_source = row
        .get::<_, Option<String>>("import_source_name")
        .ok()
        .flatten()
        .unwrap_or(row.get::<_, String>("import_source")?);
    doc.insert("importSource".into(), json!(import_source));
    doc.insert(
        "messageIdHeader".into(),
        json!(row.get::<_, Option<String>>("message_id_header")?),
    );
    doc.insert(
        "providerMessageId".into(),
        json!(row.get::<_, Option<String>>("provider_message_id")?),
    );
    doc.insert("sentAt".into(), json!(iso(row.get::<_, i64>("sent_at")?)));
    // What sentAt actually is (sent / sent_zone_unknown / received / unknown), so
    // the UI can label it truthfully. NULL (pre-backfill) → "sent", the old default.
    doc.insert(
        "sentAtKind".into(),
        json!(row.get::<_, Option<String>>("sent_at_kind")?.unwrap_or_else(|| "sent".into())),
    );
    doc.insert("subject".into(), json!(row.get::<_, Option<String>>("subject")?));
    doc.insert("senderName".into(), json!(row.get::<_, Option<String>>("sender_name")?));
    doc.insert("senderEmail".into(), json!(row.get::<_, String>("sender_email")?));
    doc.insert("recipients".into(), Value::Array(recipients));
    doc.insert("storagePath".into(), json!(row.get::<_, String>("storage_path")?));
    doc.insert(
        "storageHashSha256".into(),
        json!(row.get::<_, String>("storage_hash_sha256")?),
    );
    doc.insert("sizeBytes".into(), json!(row.get::<_, i64>("size_bytes")?));
    doc.insert("hasAttachments".into(), json!(row.get::<_, i64>("has_attachments")? != 0));
    doc.insert("archivedAt".into(), json!(iso(row.get::<_, i64>("archived_at")?)));
    doc.insert("sourcePath".into(), json!(row.get::<_, Option<String>>("source_path")?));
    for (js, col) in [
        ("duplicateSubjectHash", "duplicate_subject_hash"),
        ("duplicateBodyHash", "duplicate_body_hash"),
        ("duplicateRecipientFingerprint", "duplicate_recipient_fingerprint"),
        ("duplicateAttachmentFingerprint", "duplicate_attachment_fingerprint"),
    ] {
        doc.insert(js.into(), json!(row.get::<_, Option<String>>(col)?));
    }
    doc.insert(
        "remoteContentStatus".into(),
        json!(row.get::<_, String>("remote_content_status")?),
    );
    doc.insert(
        "remoteContentAssetCount".into(),
        json!(row.get::<_, i64>("remote_content_asset_count")?),
    );
    doc.insert(
        "remoteContentArchivedAt".into(),
        json!(row.get::<_, Option<i64>>("remote_content_archived_at")?.map(iso)),
    );
    doc.insert(
        "tags".into(),
        row.get::<_, Option<String>>("tags")?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null),
    );
    Ok(Some(doc))
}

/// Full ingestion_sources row — includes `provider_config`.
fn source_full_row(conn: &Connection, id: &str) -> Option<Value> {
    conn.query_row(
        "SELECT id, name, provider, provider_config, status, last_import_started_at, \
         last_import_finished_at, last_import_status_message, merged_into_id, \
         created_at, updated_at FROM ingestion_sources WHERE id = ?",
        [id],
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "provider_config": row.get::<_, Option<String>>(3)?,
                "status": row.get::<_, String>(4)?,
                "lastImportStartedAt": row.get::<_, Option<i64>>(5)?.map(iso),
                "lastImportFinishedAt": row.get::<_, Option<i64>>(6)?.map(iso),
                "lastImportStatusMessage": row.get::<_, Option<String>>(7)?,
                "mergedIntoId": row.get::<_, Option<String>>(8)?,
                "createdAt": iso(row.get::<_, i64>(9)?),
                "updatedAt": iso(row.get::<_, i64>(10)?),
            }))
        },
    )
    .ok()
}

// ---------------------------------------------------------------------------
// GET /archived-emails/:id — full email detail with raw bytes + thread.
// ---------------------------------------------------------------------------

pub async fn email_detail(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = app.pool.get().unwrap();
    let row = conn.query_row(
        "SELECT ae.*, COALESCE(src.name, ae.import_source) AS import_source_name \
         FROM archived_emails ae \
         LEFT JOIN ingestion_sources src ON src.id = ae.ingestion_source_id \
         WHERE ae.id = ?",
        [&id],
        |row| email_full_row(row),
    );
    let doc = match row {
        Ok(Some(doc)) => doc,
        Ok(None) => return internal_error(),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "message": "Archived email not found" })),
            )
                .into_response();
        }
        Err(_) => return internal_error(),
    };
    let mut doc = doc;

    // Relation: full ingestion source row (provider_config included).
    let source_id = doc["ingestionSourceId"].as_str().unwrap_or_default().to_string();
    doc.insert(
        "ingestionSource".into(),
        source_full_row(&conn, &source_id).unwrap_or(Value::Null),
    );

    // Thread — only when threadId is set; spans the merge group.
    let mut thread: Vec<Value> = Vec::new();
    if let Some(thread_id) = doc["threadId"].as_str() {
        let Some(group_ids) = search::group_source_ids(&conn, &source_id) else {
            return internal_error();
        };
        let placeholders = vec!["?"; group_ids.len()].join(", ");
        let sql = format!(
            "SELECT id, subject, sent_at, sender_email, has_attachments, sent_at_kind FROM archived_emails \
             WHERE thread_id = ? AND deleted_at IS NULL AND ingestion_source_id IN ({placeholders}) ORDER BY sent_at ASC"
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let mut params: Vec<String> = vec![thread_id.to_string()];
        params.extend(group_ids);
        thread = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "subject": row.get::<_, Option<String>>(1)?,
                    "sentAt": iso(row.get::<_, i64>(2)?),
                    "senderEmail": row.get::<_, String>(3)?,
                    "hasAttachments": row.get::<_, i64>(4)? != 0,
                    "sentAtKind": row.get::<_, Option<String>>(5)?.unwrap_or_else(|| "sent".into()),
                }))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();
    }

    // The raw .eml is NOT embedded here — encoding it as a JSON byte-array
    // costs ~3.5 bytes/byte on every open. The three consumers (attachment
    // parsing, copy-reply, view-headers) fetch it lazily from the dedicated
    // binary endpoint GET /archived-emails/:id/raw instead.
    doc.insert("thread".into(), Value::Array(thread));

    if doc["hasAttachments"] == Value::Bool(true) {
        let mut stmt = conn
            .prepare(
                // DISTINCT: a read-only launch never runs migrations, so an
                // archive written before 0013 can still hold duplicate links.
                "SELECT DISTINCT a.id, a.filename, a.mime_type, a.size_bytes, a.storage_path, \
                 a.content_description, a.original_created_at, a.original_modified_at \
                 FROM email_attachments ea \
                 INNER JOIN attachments a ON ea.attachment_id = a.id \
                 WHERE ea.email_id = ?",
            )
            .unwrap();
        let attachments: Vec<Value> = stmt
            .query_map([&id], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "filename": row.get::<_, String>(1)?,
                    "mimeType": row.get::<_, Option<String>>(2)?,
                    "sizeBytes": row.get::<_, i64>(3)?,
                    "storagePath": row.get::<_, String>(4)?,
                    "contentDescription": row.get::<_, Option<String>>(5)?,
                    "originalCreatedAt": row.get::<_, Option<String>>(6)?,
                    "originalModifiedAt": row.get::<_, Option<String>>(7)?,
                }))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        doc.insert("attachments".into(), Value::Array(attachments));
    }

    Json(Value::Object(doc)).into_response()
}

// ---------------------------------------------------------------------------
// GET /archived-emails/ingestion-source/:ingestionSourceId
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Dashboard: ingestion-sources, remote-content-issues
// ---------------------------------------------------------------------------

pub async fn dashboard_sources(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let mut stmt = conn
        .prepare(
            // Exclude trashed emails from the per-source counts/storage to match
            // the mailbox. The filter lives in the LEFT JOIN's ON clause (not a
            // WHERE) so a source whose only emails are trashed still appears, at 0.
            "SELECT ingestion_sources.id, ingestion_sources.name, ingestion_sources.provider, \
             ingestion_sources.status, sum(archived_emails.size_bytes), \
             count(archived_emails.id) \
             FROM ingestion_sources \
             LEFT JOIN archived_emails ON ingestion_sources.id = archived_emails.ingestion_source_id \
             AND archived_emails.deleted_at IS NULL \
             GROUP BY ingestion_sources.id",
        )
        .unwrap();
    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            // A NULL sum (source with no emails) is passed through untouched.
            let storage_used: Option<i64> = row.get(4)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "storageUsed": storage_used,
                "emailCount": row.get::<_, i64>(5)?,
            }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    Json(Value::Array(rows))
}

pub async fn remote_content_issues(
    State(app): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let page = params
        .get("page")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(1)
        .max(1);
    let limit = params
        .get("limit")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(25)
        .clamp(1, 100);
    let status = match params.get("status").map(String::as_str) {
        Some(s @ ("failed" | "partial")) => s,
        _ => "all",
    };
    let statuses: Vec<&str> = if status == "all" {
        vec!["failed", "partial"]
    } else {
        vec![status]
    };
    let sort_column = match params.get("sort").map(String::as_str) {
        Some("subject") => "subject",
        Some("status") => "remote_content_status",
        _ => "archived_at",
    };
    let direction = if params.get("direction").map(String::as_str) == Some("asc") {
        "asc"
    } else {
        "desc"
    };

    let in_clause = vec!["?"; statuses.len()].join(", ");
    let total: i64 = conn
        .query_row(
            &format!("SELECT count(*) FROM archived_emails WHERE remote_content_status IN ({in_clause})"),
            rusqlite::params_from_iter(statuses.iter()),
            |r| r.get(0),
        )
        .unwrap_or(0);

    let sql = format!(
        "SELECT id, subject, sender_name, sender_email, remote_content_status, archived_at \
         FROM archived_emails WHERE remote_content_status IN ({in_clause}) \
         ORDER BY {sort_column} {direction} LIMIT ? OFFSET ?"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    let mut qparams: Vec<rusqlite::types::Value> = statuses
        .iter()
        .map(|s| rusqlite::types::Value::from(s.to_string()))
        .collect();
    qparams.push(limit.into());
    qparams.push((page - 1).saturating_mul(limit).into());
    struct EmailRow {
        id: String,
        subject: Option<String>,
        sender_name: Option<String>,
        sender_email: String,
        status: String,
        archived_at: i64,
    }
    let emails: Vec<EmailRow> = stmt
        .query_map(rusqlite::params_from_iter(qparams.iter()), |row| {
            Ok(EmailRow {
                id: row.get(0)?,
                subject: row.get(1)?,
                sender_name: row.get(2)?,
                sender_email: row.get(3)?,
                status: row.get(4)?,
                archived_at: row.get(5)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();

    // Failed/blocked assets for the page's emails, grouped per email.
    let mut assets_by_email: HashMap<String, Vec<Value>> = HashMap::new();
    if !emails.is_empty() {
        let ids: Vec<&String> = emails.iter().map(|e| &e.id).collect();
        let in_ids = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT email_id, original_url, status, failure_reason FROM remote_content_assets \
             WHERE email_id IN ({in_ids}) AND status IN ('failed', 'blocked')"
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    json!({
                        "url": row.get::<_, String>(1)?,
                        "status": row.get::<_, String>(2)?,
                        "reason": row.get::<_, Option<String>>(3)?,
                    }),
                ))
            })
            .unwrap()
            .filter_map(Result::ok);
        for (email_id, asset) in rows {
            assets_by_email.entry(email_id).or_default().push(asset);
        }
    }

    let items: Vec<Value> = emails
        .iter()
        .map(|e| {
            json!({
                "emailId": e.id,
                "subject": match &e.subject {
                    Some(s) if !s.is_empty() => s.clone(),
                    _ => "(no subject)".to_string(),
                },
                "sender": match (&e.sender_name, &e.sender_email) {
                    (Some(n), _) if !n.is_empty() => n.clone(),
                    (_, s) if !s.is_empty() => s.clone(),
                    _ => "Unknown sender".to_string(),
                },
                "status": e.status,
                "archivedAt": iso(e.archived_at),
                "assets": assets_by_email.get(&e.id).cloned().unwrap_or_default(),
            })
        })
        .collect();

    Json(json!({ "items": items, "total": total, "page": page, "limit": limit }))
}

// ---------------------------------------------------------------------------
// GET /jobs/queues/:queueName
// ---------------------------------------------------------------------------

const QUEUE_NAMES: [&str; 3] = ["ingestion", "indexing", "remote-content"];

fn jobs_error() -> Response {
    // Body is { message, error } with error as an empty object {}.
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "message": "Error fetching queue jobs", "error": {} })),
    )
        .into_response()
}

pub async fn jobs_queue_details(
    State(app): State<AppState>,
    AxumPath(queue_name): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let conn = app.pool.get().unwrap();
    if !QUEUE_NAMES.contains(&queue_name.as_str()) {
        return jobs_error();
    }
    let now = search::now_ms();
    let status = params.get("status").cloned().unwrap_or_default();
    let predicate = match status.as_str() {
        "active" | "completed" | "failed" => format!("state = '{status}'"),
        "waiting" => format!("state = 'pending' AND run_at <= {now}"),
        "delayed" => format!("state = 'pending' AND run_at > {now}"),
        "paused" => "0 = 1".to_string(),
        _ => return jobs_error(), // unknown status → error response
    };
    let page = params
        .get("page")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p >= 1)
        .unwrap_or(1);
    // Clamp to a positive range — a negative limit is `LIMIT -1` in SQLite,
    // which disables the limit and dumps every job.
    let limit = params
        .get("limit")
        .and_then(|p| p.parse::<i64>().ok())
        .map(|l| l.clamp(1, 100))
        .unwrap_or(10);

    // Counts come from the same overview query as GET /jobs/queues.
    let (active, completed, failed, delayed, waiting): (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT \
             count(*) FILTER (WHERE state = 'active'), \
             count(*) FILTER (WHERE state = 'completed'), \
             count(*) FILTER (WHERE state = 'failed'), \
             count(*) FILTER (WHERE state = 'pending' AND run_at > (unixepoch() * 1000)), \
             count(*) FILTER (WHERE state = 'pending' AND run_at <= (unixepoch() * 1000)) \
             FROM jobs WHERE queue = ?",
            [&queue_name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap_or((0, 0, 0, 0, 0));

    let total_jobs: i64 = conn
        .query_row(
            &format!("SELECT count(*) FROM jobs WHERE queue = ? AND ({predicate})"),
            [&queue_name],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let sql = format!(
        "SELECT id, name, payload, attempts, error, created_at, started_at, finished_at \
         FROM jobs WHERE queue = ? AND ({predicate}) \
         ORDER BY created_at ASC LIMIT {limit} OFFSET {}",
        (page - 1).saturating_mul(limit)
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    let jobs: Vec<Value> = stmt
        .query_map([&queue_name], |row| {
            let payload_raw: String = row.get(2)?;
            let payload: Value = serde_json::from_str(&payload_raw).unwrap_or(Value::Null);
            let payload = if payload.is_null() { json!({}) } else { payload };
            let error: Option<String> = row.get(4)?;
            let started_at: Option<i64> = row.get(6)?;
            let finished_at: Option<i64> = row.get(7)?;
            let mut job = Map::new();
            job.insert("id".into(), json!(row.get::<_, String>(0)?));
            job.insert("name".into(), json!(row.get::<_, String>(1)?));
            job.insert("data".into(), payload.clone());
            job.insert("state".into(), json!(status));
            // Only included when there's an error.
            if let Some(err) = &error {
                job.insert(
                    "failedReason".into(),
                    json!(err.split('\n').next().unwrap_or("")),
                );
            }
            job.insert("timestamp".into(), json!(row.get::<_, i64>(5)?));
            if let Some(ms) = started_at {
                job.insert("processedOn".into(), json!(ms));
            }
            if let Some(ms) = finished_at {
                job.insert("finishedOn".into(), json!(ms));
            }
            job.insert("attemptsMade".into(), json!(row.get::<_, i64>(3)?));
            job.insert(
                "stacktrace".into(),
                match &error {
                    Some(err) if !err.is_empty() => json!([err]),
                    _ => json!([]),
                },
            );
            job.insert("returnValue".into(), Value::Null);
            if let Some(source_id) = payload.get("ingestionSourceId") {
                job.insert("ingestionSourceId".into(), source_id.clone());
            }
            if status == "failed" {
                job.insert("error".into(), json!(error));
            }
            Ok(Value::Object(job))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();

    let total_pages = if total_jobs == 0 {
        0
    } else {
        (total_jobs + limit - 1) / limit
    };
    Json(json!({
        "name": queue_name,
        "counts": {
            "active": active,
            "completed": completed,
            "failed": failed,
            "delayed": delayed,
            "waiting": waiting,
            "paused": 0,
        },
        "jobs": jobs,
        "pagination": {
            "currentPage": page,
            "totalPages": total_pages,
            "totalJobs": total_jobs,
            "limit": limit,
        },
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /ingestion-sources/:id
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// GET /storage/download?path=...
// ---------------------------------------------------------------------------

/// Builds a text/html string response — used for error bodies.
fn text_response(status: StatusCode, body: &'static str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Materializes the storage-relative attachment at `unsafe_path` into the OS
/// temp dir for an external previewer, scheduling best-effort removal after 10
/// minutes. Returns the temp file path; errors are (status, message) pairs so
/// both the HTTP handler below and the desktop shell's native Quick Look
/// endpoint can map them directly.
pub fn materialize_quicklook_temp(
    app: &AppState,
    unsafe_path: &str,
) -> Result<std::path::PathBuf, (StatusCode, &'static str)> {
    if unsafe_path.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "File path is required"));
    }
    let file = app
        .storage_abs(unsafe_path)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid file path"))?;
    if !file.is_file() {
        return Err((StatusCode::NOT_FOUND, "File not found"));
    }
    let content = std::fs::read(&file)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Error reading file"))?;

    // The storage basename is already `<hash7>-<sanitized original name>`, so
    // it is collision-free and keeps the extension the previewer needs.
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment")
        .to_string();
    let dir = std::env::temp_dir().join("pea-quicklook");
    std::fs::create_dir_all(&dir)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "An internal server error occurred"))?;
    let target = dir.join(name);
    std::fs::write(&target, &content)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "An internal server error occurred"))?;

    // Best-effort cleanup once the previewer has had time to read it.
    let cleanup = target.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(600));
        let _ = std::fs::remove_file(&cleanup);
    });
    Ok(target)
}

/// POST /attachments/quicklook {path} — opens the attachment in the
/// OS quick-look previewer (qlmanage on macOS, sushi/xdg-open on Linux).
/// Desktop-only by nature: the file is materialized in the OS temp dir for
/// the previewer and removed again a few minutes later. The macOS desktop
/// shell presents a native QLPreviewPanel via /api/v1/native/quicklook
/// instead; this endpoint is the web/Linux fallback.
pub async fn quicklook_attachment(
    State(app): State<AppState>,
    Json(body): Json<Value>,
) -> Response {
    let unsafe_path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let target = match materialize_quicklook_temp(&app, unsafe_path) {
        Ok(target) => target,
        Err((status, message)) => return text_response(status, message),
    };

    let quiet = |mut cmd: std::process::Command| {
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    };
    let spawned = if cfg!(target_os = "macos") {
        let mut cmd = std::process::Command::new("qlmanage");
        cmd.arg("-p").arg(&target);
        quiet(cmd)
    } else {
        let mut cmd = std::process::Command::new("sushi");
        cmd.arg(&target);
        quiet(cmd).or_else(|_| {
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(&target);
            quiet(cmd)
        })
    };
    if spawned.is_err() {
        let _ = std::fs::remove_file(&target);
        return text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No preview application available",
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

/// GET /archived-emails/:id/eml — the downloadable .eml, reconstructed from
/// the hollowed stored copy by splicing each attachment blob back in place.
/// Pre-hollowing emails have no markers and download exactly as stored.
pub async fn download_email_eml(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let stored_path = match conn_email_storage_path(&app, &id) {
        Ok(p) => p,
        Err(StatusCode::NOT_FOUND) => return text_response(StatusCode::NOT_FOUND, "File not found"),
        Err(code) => return text_response(code, "Error reading email"),
    };
    let Ok(stored) = app.storage_get(&stored_path) else {
        return text_response(StatusCode::INTERNAL_SERVER_ERROR, "Error downloading file");
    };

    // sha-256 → blob storage path for this email's attachments.
    let conn = app.pool.get().unwrap();
    let mut blob_paths: HashMap<String, String> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT a.content_hash_sha256, a.storage_path FROM email_attachments ea \
         INNER JOIN attachments a ON ea.attachment_id = a.id WHERE ea.email_id = ?",
    ) {
        let rows = stmt
            .query_map([&id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
            .unwrap_or_default();
        for (hash, path) in rows {
            blob_paths.insert(hash, path);
        }
    }

    let rebuilt = crate::ingest::rebuild_eml(&stored, &|hash| {
        blob_paths.get(hash).and_then(|path| app.storage_get(path).ok())
    });
    (
        [
            (header::CONTENT_TYPE, "message/rfc822".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"email.eml\"".to_string(),
            ),
        ],
        rebuilt,
    )
        .into_response()
}

/// Ok(path), Err(NOT_FOUND) when the email doesn't exist, Err(INTERNAL...) for a
/// real failure (pool exhausted, DB error) — so callers don't turn a 500 into 404.
fn conn_email_storage_path(app: &AppState, email_id: &str) -> Result<String, StatusCode> {
    let conn = app.pool.get().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match conn.query_row(
        "SELECT storage_path FROM archived_emails WHERE id = ?",
        [email_id],
        |r| r.get::<_, String>(0),
    ) {
        Ok(p) => Ok(p),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// GET /archived-emails/:id/raw — the stored (hollowed) .eml bytes, fetched
/// lazily by the detail page instead of being embedded in the JSON response.
pub async fn download_email_raw(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let storage_path = match conn_email_storage_path(&app, &id) {
        Ok(p) => p,
        Err(StatusCode::NOT_FOUND) => return text_response(StatusCode::NOT_FOUND, "File not found"),
        Err(code) => return text_response(code, "Error reading email"),
    };
    match app.storage_get(&storage_path) {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(_) => text_response(StatusCode::INTERNAL_SERVER_ERROR, "Error reading file"),
    }
}

/// Reduces an attacker-controlled attachment name to a safe zip entry name:
/// basename only, with separators and `..` neutralised (anti Zip Slip).
fn zip_entry_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty() || base == "." || base == ".." {
        "attachment".to_string()
    } else {
        base.to_string()
    }
}

/// GET /archived-emails/:id/attachments/archive — all of an email's
/// attachments bundled into one zip.
pub async fn download_all_attachments(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = app.pool.get().unwrap();
    let mut stmt = match conn.prepare(
        // DISTINCT, or a duplicate link would add the same file to the zip twice
        // — the second copy renamed by the collision loop below, planting a
        // filename the email never contained.
        "SELECT DISTINCT a.filename, a.storage_path FROM email_attachments ea \
         INNER JOIN attachments a ON ea.attachment_id = a.id WHERE ea.email_id = ?",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return internal_error(),
    };
    let attachments: Vec<(String, String)> = stmt
        .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    if attachments.is_empty() {
        return text_response(StatusCode::NOT_FOUND, "File not found");
    }

    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut cursor);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (raw_filename, storage_path) in attachments {
        let Ok(content) = app.storage_get(&storage_path) else {
            return text_response(StatusCode::INTERNAL_SERVER_ERROR, "Error downloading file");
        };
        // The DB filename is the raw MIME attachment name (attacker-controlled),
        // so reduce it to a bare basename before it becomes a zip entry — a
        // path like `../../evil` would otherwise plant files on extraction.
        let base = zip_entry_name(&raw_filename);
        // Suffix before the extension until the ACTUAL emitted name is unused,
        // so a generated `a-2.pdf` can't collide with a real `a-2.pdf`.
        let mut entry_name = base.clone();
        let mut n = 1;
        while used_names.contains(&entry_name) {
            n += 1;
            entry_name = match base.rsplit_once('.') {
                Some((stem, ext)) => format!("{stem}-{n}.{ext}"),
                None => format!("{base}-{n}"),
            };
        }
        used_names.insert(entry_name.clone());
        if writer.start_file(entry_name, options).is_err() {
            return internal_error();
        }
        if std::io::Write::write_all(&mut writer, &content).is_err() {
            return internal_error();
        }
    }
    if writer.finish().is_err() {
        return internal_error();
    }

    (
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"attachments.zip\"".to_string(),
            ),
        ],
        cursor.into_inner(),
    )
        .into_response()
}

pub async fn storage_download(
    State(app): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(unsafe_path) = params.get("path").filter(|p| !p.is_empty()) else {
        return text_response(StatusCode::BAD_REQUEST, "File path is required");
    };
    let file = match app.storage_abs(unsafe_path) {
        Ok(file) => file,
        Err(_) => return text_response(StatusCode::BAD_REQUEST, "Invalid file path"),
    };
    if !file.is_file() {
        return text_response(StatusCode::NOT_FOUND, "File not found");
    }
    let content = match std::fs::read(&file) {
        Ok(content) => content,
        _ => return text_response(StatusCode::INTERNAL_SERVER_ERROR, "Error downloading file"),
    };
    let filename = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    (
        [(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )],
        content,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_entry_name_basenames_and_defaults() {
        assert_eq!(zip_entry_name("../../evil.sh"), "evil.sh");
        assert_eq!(zip_entry_name("a/b/c.pdf"), "c.pdf");
        assert_eq!(zip_entry_name("plain.txt"), "plain.txt");
        assert_eq!(zip_entry_name(""), "attachment");
        assert_eq!(zip_entry_name(".."), "attachment");
    }
}
