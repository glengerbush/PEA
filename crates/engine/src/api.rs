//! HTTP surface — read handlers that live at the router root, plus the
//! router assembly (each nested under /v1 and /api/v1, like the Node server).

use crate::state::AppState;
use crate::{duplicates, handlers, preview, writes};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post, put};
use axum::Router;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::iso;
use crate::search;

fn qlookup(params: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
    move |key: &str| params.get(key).cloned()
}

async fn archived_emails(
    State(app): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let started = search::now_ms();
    let conn = app.pool.get().unwrap();
    Json(search::query_archived_emails(&conn, &qlookup(&params), started))
}

async fn facets(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    Json(search::filter_facets(&conn))
}

async fn dashboard_stats(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let total_emails: i64 = conn
        .query_row("SELECT count(*) FROM archived_emails", [], |r| r.get(0))
        .unwrap_or(0);
    let total_storage: i64 = conn
        .query_row("SELECT COALESCE(sum(size_bytes), 0) FROM archived_emails", [], |r| r.get(0))
        .unwrap_or(0);
    let seven_days_ago = search::now_ms() - 7 * 24 * 3600 * 1000;
    let failed7: i64 = conn
        .query_row(
            "SELECT count(*) FROM ingestion_sources WHERE status = 'error' AND updated_at >= ?",
            [seven_days_ago],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let (rc_failed, rc_partial): (i64, i64) = conn
        .query_row(
            "SELECT count(*) FILTER (WHERE remote_content_status = 'failed'), \
             count(*) FILTER (WHERE remote_content_status = 'partial') FROM archived_emails",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    Json(json!({
        "totalEmailsArchived": total_emails,
        "totalStorageUsed": total_storage,
        "failedIngestionsLast7Days": failed7,
        "remoteContentFailed": rc_failed,
        "remoteContentPartial": rc_partial,
    }))
}

async fn dashboard_history(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let thirty_days_ago = search::now_ms() - 30 * 24 * 3600 * 1000;
    let mut stmt = conn
        .prepare(
            "SELECT date(archived_at / 1000, 'unixepoch') AS d, count(*) AS c \
             FROM archived_emails WHERE archived_at >= ? GROUP BY d ORDER BY d",
        )
        .unwrap();
    let history: Vec<Value> = stmt
        .query_map([thirty_days_ago], |row| {
            Ok(json!({ "date": row.get::<_, String>(0)?, "count": row.get::<_, i64>(1)? }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    Json(json!({ "history": history }))
}

async fn dashboard_insights(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    Json(json!({ "topSenders": search::top_senders(&conn, 10) }))
}

async fn settings_system(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    // SettingsService.getSystemSettings: defaults ← stored config, creating the
    // default record when missing (best-effort in read-only mode).
    let mut merged: Value = json!({
        "language": "en",
        "theme": "system",
        "timeZone": Value::Null,
        "clockFormat": "12h",
        "autoCheckUpdates": true,
    });
    let config: Option<String> = conn
        .query_row("SELECT config FROM system_settings LIMIT 1", [], |r| r.get(0))
        .ok();
    match config {
        Some(stored) => {
            if let Some(stored) = serde_json::from_str::<Value>(&stored).ok().and_then(|v| {
                v.as_object().cloned()
            }) {
                let m = merged.as_object_mut().unwrap();
                for (k, v) in stored {
                    m.insert(k, v);
                }
            }
        }
        None => {
            // id is INTEGER AUTOINCREMENT — let SQLite assign it.
            conn.execute(
                "INSERT INTO system_settings (config) VALUES (?)",
                rusqlite::params![merged.to_string()],
            )
            .ok();
        }
    }
    Json(merged)
}

async fn contacts_map(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let mut stmt = conn
        .prepare("SELECT email, display_name FROM contacts")
        .unwrap();
    let mut map = serde_json::Map::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();
    for row in rows.filter_map(Result::ok) {
        map.insert(row.0, Value::String(row.1));
    }
    Json(Value::Object(map))
}

async fn ingestion_sources(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, provider, status, last_import_started_at, \
             last_import_finished_at, last_import_status_message, merged_into_id, \
             created_at, updated_at FROM ingestion_sources ORDER BY created_at DESC",
        )
        .unwrap();
    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "lastImportStartedAt": row.get::<_, Option<i64>>(4)?.map(iso),
                "lastImportFinishedAt": row.get::<_, Option<i64>>(5)?.map(iso),
                "lastImportStatusMessage": row.get::<_, Option<String>>(6)?,
                "mergedIntoId": row.get::<_, Option<String>>(7)?,
                "createdAt": iso(row.get::<_, i64>(8)?),
                "updatedAt": iso(row.get::<_, i64>(9)?),
            }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    Json(Value::Array(rows))
}

async fn jobs_queues(State(app): State<AppState>) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    let mut by_queue: HashMap<String, (i64, i64, i64, i64, i64)> = HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT queue, \
             count(*) FILTER (WHERE state = 'active'), \
             count(*) FILTER (WHERE state = 'completed'), \
             count(*) FILTER (WHERE state = 'failed'), \
             count(*) FILTER (WHERE state = 'pending' AND run_at > (unixepoch() * 1000)), \
             count(*) FILTER (WHERE state = 'pending' AND run_at <= (unixepoch() * 1000)) \
             FROM jobs GROUP BY queue",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?),
            ))
        })
        .unwrap();
    for row in rows.filter_map(Result::ok) {
        by_queue.insert(row.0, row.1);
    }
    let queues: Vec<Value> = ["ingestion", "indexing", "remote-content"]
        .iter()
        .map(|name| {
            let (active, completed, failed, delayed, waiting) =
                by_queue.get(*name).copied().unwrap_or((0, 0, 0, 0, 0));
            json!({
                "name": name,
                "counts": {
                    "active": active,
                    "completed": completed,
                    "failed": failed,
                    "delayed": delayed,
                    "waiting": waiting,
                    "paused": 0,
                }
            })
        })
        .collect();
    Json(json!({ "queues": queues }))
}

async fn duplicates_exact(
    State(app): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    Json(duplicates::list_exact_groups(
        &conn,
        params.get("page").map(String::as_str),
        params.get("limit").map(String::as_str),
        params.get("reason").map(String::as_str),
    ))
}

async fn duplicates_likely(
    State(app): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let conn = app.pool.get().unwrap();
    Json(duplicates::list_likely_duplicates(
        &conn,
        params.get("page").map(String::as_str),
        params.get("limit").map(String::as_str),
    ))
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        // Express (non-strict routing) matched both forms; the mailbox page
        // fetches the list WITHOUT the trailing slash, so serve both.
        .route("/archived-emails", get(archived_emails))
        .route("/archived-emails/", get(archived_emails))
        .route("/archived-emails/facets", get(facets))
        .route("/archived-emails/duplicates/exact", get(duplicates_exact))
        .route("/archived-emails/duplicates/likely", get(duplicates_likely))
        .route(
            "/archived-emails/duplicates/exact/approve",
            post(writes::approve_exact_duplicates),
        )
        .route("/archived-emails/duplicates/likely/approve", post(writes::approve_likely))
        .route("/archived-emails/duplicates/likely/ignore", post(writes::ignore_likely))
        .route("/archived-emails/bulk/tags", post(writes::update_tags))
        .route("/archived-emails/bulk/delete", post(writes::bulk_delete))
        .route(
            "/archived-emails/{id}",
            get(handlers::email_detail).delete(writes::delete_email),
        )
        .route("/archived-emails/{id}/preview", get(preview::email_preview))
        .route(
            "/archived-emails/{id}/attachments/archive",
            get(handlers::download_all_attachments),
        )
        .route("/archived-emails/{id}/eml", get(handlers::download_email_eml))
        .route("/archived-emails/{id}/raw", get(handlers::download_email_raw))
        .route(
            "/attachments/quicklook",
            post(handlers::quicklook_attachment),
        )
        .route(
            "/archived-emails/{id}/remote-content/archive",
            post(writes::enqueue_remote_content),
        )
        .route(
            "/archived-emails/{id}/remote-assets",
            get(preview::list_remote_assets),
        )
        .route(
            "/archived-emails/{id}/remote-assets/{assetId}",
            get(preview::get_remote_asset),
        )
        .route("/dashboard/stats", get(dashboard_stats))
        .route("/dashboard/ingestion-history", get(dashboard_history))
        .route("/dashboard/ingestion-sources", get(handlers::dashboard_sources))
        .route("/dashboard/indexed-insights", get(dashboard_insights))
        .route(
            "/dashboard/remote-content-issues",
            get(handlers::remote_content_issues),
        )
        .route("/settings/system", get(settings_system).put(writes::update_settings))
        .route("/contacts/map", get(contacts_map))
        .route("/contacts/import", post(writes::import_contacts))
        .route(
            "/ingestion-sources",
            get(ingestion_sources).post(writes::create_source),
        )
        .route(
            "/ingestion-sources/{id}",
            put(writes::update_source).delete(writes::delete_source),
        )
        .route("/ingestion-sources/{id}/pause", post(writes::pause_source))
        .route("/ingestion-sources/{id}/reimport", post(writes::reimport))
        .route("/ingestion-sources/{id}/unmerge", post(writes::unmerge_source))
        .route("/storage/download", get(handlers::storage_download))
        .route("/jobs/queues", get(jobs_queues))
        .route("/jobs/queues/{queueName}", get(handlers::jobs_queue_details));
    Router::new()
        .nest("/api/v1", api)
        .fallback(static_files)
        .with_state(state)
}

fn content_type_for(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// SPA static serving — the Rust twin of the Node bootstrap's express.static +
/// index.html fallback (API paths excluded). Active only when a frontend build
/// dir is configured (the desktop shell and --serve mode set it).
async fn static_files(
    State(app): State<AppState>,
    uri: axum::http::Uri,
) -> axum::response::Response {
    let Some(dir) = app.frontend_dir.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Lexically sanitize — no parent traversal out of the build dir.
    let safe: std::path::PathBuf = std::path::Path::new(path)
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .collect();
    let mut file = dir.join(&safe);
    if !file.is_file() {
        file = dir.join("index.html"); // SPA fallback
    }
    match tokio::fs::read(&file).await {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, content_type_for(&file))],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

