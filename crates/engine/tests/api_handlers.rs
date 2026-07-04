//! HTTP handler smoke tests. Builds the real axum router over a temp archive
//! (provisioned + populated from the golden mbox) and drives it with
//! `tower::ServiceExt::oneshot` — no socket, no reqwest. This covers the
//! request→handler→DB path that the ingest smoke test does not touch, and
//! exercises the contacts write path against the current (post-reclaim) schema.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use std::path::PathBuf;
use tower::ServiceExt;

/// Send one request through the router and return (status, raw body bytes).
async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Vec<u8>) {
    let builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, bytes.to_vec())
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    let (status, bytes) = send(app, "GET", uri, None).await;
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn api_handlers_smoke() {
    // ---- setup: fresh archive populated from the golden fixture ----
    let tmp = std::env::temp_dir().join(format!("pea-api-smoke-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    std::fs::create_dir_all(&tmp).unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fixtures/golden.mbox");
    pea_engine::provision::provision(&tmp).expect("provision");
    pea_engine::ingest::import_mbox(&tmp, &fixture, None).expect("import");

    // Grab a real email id straight from the DB before the state moves into the router.
    let state = pea_engine::state_for_dir(&tmp, false).expect("state");
    let some_id: String = {
        let conn = state.pool.get().unwrap();
        conn.query_row("SELECT id FROM archived_emails LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let app = pea_engine::api::router(state);

    // ---- dashboard/stats: reflects the 7 archived emails ----
    let (s, body) = get_json(&app, "/api/v1/dashboard/stats").await;
    assert_eq!(s, StatusCode::OK, "dashboard/stats status");
    assert_eq!(body["totalEmailsArchived"], json!(7), "stats email count");
    assert!(body["totalStorageUsed"].as_i64().unwrap() > 0, "stats storage");

    // ---- archived-emails list (FTS query path): total == 7, hits populated ----
    let (s, body) = get_json(&app, "/api/v1/archived-emails?limit=50").await;
    assert_eq!(s, StatusCode::OK, "list status");
    assert_eq!(body["total"], json!(7), "list total");
    assert_eq!(body["hits"].as_array().unwrap().len(), 7, "list hits");

    // ---- full-text search on page 1: must return BOTH hits and a total.
    //      Regression guard: COUNT(*) OVER () folded next to bm25()/snippet()
    //      raised "unable to use function bm25" and silently returned 0 here. ----
    let (s, body) = get_json(&app, "/api/v1/archived-emails?q=reprap").await;
    assert_eq!(s, StatusCode::OK, "search status");
    assert_eq!(body["total"], json!(2), "search 'reprap' total");
    assert_eq!(
        body["hits"].as_array().unwrap().len(),
        2,
        "search 'reprap' returns hits on page 1"
    );

    // ---- facets ----
    let (s, _) = get_json(&app, "/api/v1/archived-emails/facets").await;
    assert_eq!(s, StatusCode::OK, "facets status");

    // ---- single email detail ----
    let (s, body) = get_json(&app, &format!("/api/v1/archived-emails/{some_id}")).await;
    assert_eq!(s, StatusCode::OK, "detail status");
    assert_eq!(body["id"], json!(some_id), "detail id echoes");

    // ---- raw + eml download (binary bodies) ----
    let (s, bytes) = send(&app, "GET", &format!("/api/v1/archived-emails/{some_id}/raw"), None).await;
    assert_eq!(s, StatusCode::OK, "raw status");
    assert!(!bytes.is_empty(), "raw body non-empty");
    let (s, bytes) = send(&app, "GET", &format!("/api/v1/archived-emails/{some_id}/eml"), None).await;
    assert_eq!(s, StatusCode::OK, "eml status");
    assert!(bytes.windows(5).any(|w| w == b"From:"), "eml is a real message");

    // ---- unknown id → 404 (not a 500) ----
    let (s, _) = get_json(&app, "/api/v1/archived-emails/does-not-exist").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "unknown id is 404");

    // ---- duplicates listing ----
    let (s, _) = get_json(&app, "/api/v1/archived-emails/duplicates/exact").await;
    assert_eq!(s, StatusCode::OK, "duplicates/exact status");

    // ---- settings, profile, ingestion sources, jobs ----
    let (s, body) = get_json(&app, "/api/v1/settings/system").await;
    assert_eq!(s, StatusCode::OK, "settings status");
    assert_eq!(body["theme"], json!("system"), "settings default theme");

    let (s, body) = get_json(&app, "/api/v1/ingestion-sources").await;
    assert_eq!(s, StatusCode::OK, "sources status");
    assert_eq!(body.as_array().unwrap().len(), 1, "one imported source");

    let (s, body) = get_json(&app, "/api/v1/jobs/queues").await;
    assert_eq!(s, StatusCode::OK, "jobs status");
    assert_eq!(body["queues"].as_array().unwrap().len(), 3, "three queues");

    // ---- contacts import (write path against the reclaimed 3-column schema) ----
    let (s, body) = get_json(&app, "/api/v1/contacts/map").await;
    assert_eq!(s, StatusCode::OK, "contacts/map status");
    assert_eq!(body.as_object().unwrap().len(), 0, "no contacts yet");

    let csv = "email,name\nalice@example.com,Alice A\nbob@example.com,Bob B\n";
    let (s, body) = send(
        &app,
        "POST",
        "/api/v1/contacts/import",
        Some(json!({ "format": "csv", "content": csv })),
    )
    .await;
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    assert_eq!(s, StatusCode::OK, "contacts import status (schema still valid)");
    assert_eq!(body["imported"], json!(2), "two contacts imported");

    let (s, body) = get_json(&app, "/api/v1/contacts/map").await;
    assert_eq!(s, StatusCode::OK, "contacts/map after import");
    assert_eq!(body["alice@example.com"], json!("Alice A"), "imported contact resolves");

    // ---- bad request: missing format ----
    let (s, _) = send(
        &app,
        "POST",
        "/api/v1/contacts/import",
        Some(json!({ "content": "x" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "missing format → 400");

    std::fs::remove_dir_all(&tmp).ok();
}
