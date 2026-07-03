//! R1 read handlers beyond search: email detail / per-source listing,
//! dashboard summaries, queue details, source detail, storage download.
//! Every shape mirrors the Node engine byte-for-byte (key order aside).

use crate::state::AppState;
use crate::{crypto, iso, search};
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

/// mapRecipients — flattens {to,cc,bcc} into [{name?, email?}]. `None` mirrors
/// the Node TypeError (destructuring null) that surfaces as a 500.
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
            // JSON.stringify drops undefined: keys appear only when present.
            if let Some(name) = r.get("name") {
                entry.insert("name".into(), name.clone());
            }
            if let Some(address) = r.get("address") {
                entry.insert("email".into(), address.clone());
            }
            out.push(Value::Object(entry));
        }
    }
    Some(out)
}

/// Full drizzle archived_emails row shape (as spread by the Node services),
/// with the controller-level overrides applied (recipients flattened,
/// sourceLabels/tags parsed-or-null, path empty→null).
fn email_full_row(row: &rusqlite::Row) -> rusqlite::Result<Option<Map<String, Value>>> {
    let recipients_raw: Option<String> = row.get("recipients")?;
    let Some(recipients) = flatten_recipients(recipients_raw.as_deref()) else {
        return Ok(None); // Node throws here → 500
    };
    let mut doc = Map::new();
    doc.insert("id".into(), json!(row.get::<_, String>("id")?));
    doc.insert("threadId".into(), json!(row.get::<_, Option<String>>("thread_id")?));
    doc.insert(
        "ingestionSourceId".into(),
        json!(row.get::<_, String>("ingestion_source_id")?),
    );
    doc.insert("userEmail".into(), json!(row.get::<_, String>("user_email")?));
    doc.insert(
        "messageIdHeader".into(),
        json!(row.get::<_, Option<String>>("message_id_header")?),
    );
    doc.insert(
        "providerMessageId".into(),
        json!(row.get::<_, Option<String>>("provider_message_id")?),
    );
    doc.insert("sentAt".into(), json!(iso(row.get::<_, i64>("sent_at")?)));
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
    doc.insert("isIndexed".into(), json!(row.get::<_, i64>("is_indexed")? != 0));
    doc.insert("hasAttachments".into(), json!(row.get::<_, i64>("has_attachments")? != 0));
    doc.insert("archivedAt".into(), json!(iso(row.get::<_, i64>("archived_at")?)));
    doc.insert("sourcePath".into(), json!(row.get::<_, Option<String>>("source_path")?));
    // (parsed as string[] | null) || null — a parsed empty array stays [].
    doc.insert(
        "sourceLabels".into(),
        row.get::<_, Option<String>>("source_labels")?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null),
    );
    for (js, col) in [
        ("duplicateSubjectHash", "duplicate_subject_hash"),
        ("duplicateFuzzyGroupKey", "duplicate_fuzzy_group_key"),
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
    // item.path || null — empty string is falsy.
    doc.insert(
        "path".into(),
        match row.get::<_, Option<String>>("path")? {
            Some(p) if !p.is_empty() => json!(p),
            _ => Value::Null,
        },
    );
    doc.insert(
        "tags".into(),
        row.get::<_, Option<String>>("tags")?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null),
    );
    Ok(Some(doc))
}

/// Full ingestion_sources row (drizzle mapping) — includes `credentials`,
/// exactly like the relation spread in the Node email-detail response.
fn source_full_row(conn: &Connection, id: &str) -> Option<Value> {
    conn.query_row(
        "SELECT id, user_id, name, provider, credentials, status, last_sync_started_at, \
         last_sync_finished_at, last_sync_status_message, sync_state, merged_into_id, \
         created_at, updated_at FROM ingestion_sources WHERE id = ?",
        [id],
        |row| {
            let sync_state: Option<String> = row.get(9)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "userId": row.get::<_, Option<String>>(1)?,
                "name": row.get::<_, String>(2)?,
                "provider": row.get::<_, String>(3)?,
                "credentials": row.get::<_, Option<String>>(4)?,
                "status": row.get::<_, String>(5)?,
                "lastSyncStartedAt": row.get::<_, Option<i64>>(6)?.map(iso),
                "lastSyncFinishedAt": row.get::<_, Option<i64>>(7)?.map(iso),
                "lastSyncStatusMessage": row.get::<_, Option<String>>(8)?,
                "syncState": sync_state.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
                "mergedIntoId": row.get::<_, Option<String>>(10)?,
                "createdAt": iso(row.get::<_, i64>(11)?),
                "updatedAt": iso(row.get::<_, i64>(12)?),
            }))
        },
    )
    .ok()
}

/// toSafeIngestionSource — the same row without credentials.
fn source_safe_row(conn: &Connection, id: &str) -> Option<Value> {
    let mut value = source_full_row(conn, id)?;
    value.as_object_mut()?.remove("credentials");
    Some(value)
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
        "SELECT * FROM archived_emails WHERE id = ?",
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

    // Relation: full ingestion source row (credentials included, as in Node).
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
            "SELECT id, subject, sent_at, sender_email, has_attachments FROM archived_emails \
             WHERE thread_id = ? AND ingestion_source_id IN ({placeholders}) ORDER BY sent_at ASC"
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
                }))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();
    }

    // raw .eml — read + decrypt, serialized like a Node Buffer.
    let storage_path = doc["storagePath"].as_str().unwrap_or_default().to_string();
    let file = app.storage_root().join(&storage_path);
    let raw = match std::fs::read(&file).map(|c| crypto::decrypt_storage(c, &app.storage_key)) {
        Ok(Ok(plain)) => plain,
        _ => return internal_error(), // Node's storage.get throws → 500
    };
    doc.insert("raw".into(), json!({ "type": "Buffer", "data": raw }));
    doc.insert("thread".into(), Value::Array(thread));

    if doc["hasAttachments"] == Value::Bool(true) {
        let mut stmt = conn
            .prepare(
                "SELECT a.id, a.filename, a.mime_type, a.size_bytes, a.storage_path \
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

pub async fn emails_by_source(
    State(app): State<AppState>,
    AxumPath(source_id): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let conn = app.pool.get().unwrap();
    let page = params
        .get("page")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(1);
    let limit = params
        .get("limit")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(10);
    let offset = (page - 1) * limit;

    // findGroupSourceIds throws for unknown sources → Node responds 500.
    let Some(group_ids) = search::group_source_ids(&conn, &source_id) else {
        return internal_error();
    };
    let placeholders = vec!["?"; group_ids.len()].join(", ");

    let total: i64 = {
        let sql = format!(
            "SELECT count(archived_emails.id) FROM archived_emails \
             LEFT JOIN ingestion_sources ON archived_emails.ingestion_source_id = ingestion_sources.id \
             WHERE archived_emails.ingestion_source_id IN ({placeholders})"
        );
        conn.query_row(&sql, rusqlite::params_from_iter(group_ids.iter()), |r| r.get(0))
            .unwrap_or(0)
    };

    let sql = format!(
        "SELECT archived_emails.* FROM archived_emails \
         LEFT JOIN ingestion_sources ON archived_emails.ingestion_source_id = ingestion_sources.id \
         WHERE archived_emails.ingestion_source_id IN ({placeholders}) \
         ORDER BY archived_emails.sent_at DESC LIMIT ? OFFSET ?"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    let mut params_vec: Vec<rusqlite::types::Value> = group_ids
        .iter()
        .map(|s| rusqlite::types::Value::from(s.clone()))
        .collect();
    params_vec.push(limit.into());
    params_vec.push(offset.into());
    let rows: Vec<Option<Map<String, Value>>> = stmt
        .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| email_full_row(row))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    let mut items: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        match row {
            Some(doc) => items.push(Value::Object(doc)),
            None => return internal_error(), // null recipients → Node throws
        }
    }

    Json(json!({ "items": items, "total": total, "page": page, "limit": limit })).into_response()
}

// ---------------------------------------------------------------------------
// Dashboard: ingestion-sources, recent-syncs, remote-content-issues
// ---------------------------------------------------------------------------

pub async fn dashboard_sources(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT ingestion_sources.id, ingestion_sources.name, ingestion_sources.provider, \
             ingestion_sources.status, sum(archived_emails.size_bytes) \
             FROM ingestion_sources \
             LEFT JOIN archived_emails ON ingestion_sources.id = archived_emails.ingestion_source_id \
             GROUP BY ingestion_sources.id",
        )
        .unwrap();
    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            // drizzle's mapWith(Number) passes NULL through untouched.
            let storage_used: Option<i64> = row.get(4)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "storageUsed": storage_used,
            }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    Json(Value::Array(rows))
}

pub async fn recent_syncs() -> Json<Value> {
    // Placeholder in Node too — no sync-session table yet.
    Json(json!([]))
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
    qparams.push(((page - 1) * limit).into());
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
    // res.json({ message, error }) — an Error serializes to {}.
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
        _ => return jobs_error(), // statusPredicate throws on unknown status
    };
    let page = params
        .get("page")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(1);
    let limit = params
        .get("limit")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p != 0)
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
        (page - 1) * limit
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
            // undefined keys are dropped by JSON.stringify — mirror that.
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

pub async fn ingestion_source_detail(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = app.pool.get().unwrap();
    match source_safe_row(&conn, &id) {
        Some(source) => Json(source).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "Ingestion source not found" })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /storage/download?path=...
// ---------------------------------------------------------------------------

/// Lexical normalize + strip of leading `../` — the combined effect of Node's
/// path.normalize + the traversal-stripping regex + path.relative round-trip.
fn sanitize_storage_path(unsafe_path: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for seg in unsafe_path.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                stack.pop(); // leading ..s vanish (regex), inner ..s resolve
            }
            s => stack.push(s),
        }
    }
    stack.join("/")
}

/// Express res.send(string) responds text/html — match it on error bodies.
fn text_response(status: StatusCode, body: &'static str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
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
    let safe_path = sanitize_storage_path(unsafe_path);
    let file = app.storage_root().join(&safe_path);
    if !file.is_file() {
        return text_response(StatusCode::NOT_FOUND, "File not found");
    }
    let content = match std::fs::read(&file).map(|c| crypto::decrypt_storage(c, &app.storage_key)) {
        Ok(Ok(plain)) => plain,
        _ => return text_response(StatusCode::INTERNAL_SERVER_ERROR, "Error downloading file"),
    };
    let filename = safe_path.rsplit('/').next().unwrap_or("").to_string();
    (
        [(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )],
        content,
    )
        .into_response()
}
