//! Shared test harness. Provisions a self-cleaning temp archive, imports
//! crafted mbox/MIME fixtures through the real ingest pipeline, and drives the
//! real axum router over it via `tower::oneshot` (no socket). Behavioral tests
//! assert intended behavior against this; where the engine disagrees, that's a
//! bug to fix — not an expectation to encode.
#![allow(dead_code, unused_imports)]

use axum::body::Body;
use axum::http::Request;
use axum::Router;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tower::ServiceExt;

// Re-exported for test ergonomics.
pub use axum::http::StatusCode;
pub use serde_json::json;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn uniq() -> u32 {
    COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// A provisioned temp archive dir that deletes itself on drop.
pub struct TempArchive {
    pub dir: PathBuf,
}

impl TempArchive {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("pea-test-{}-{}", std::process::id(), uniq()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        pea_engine::provision::provision(&dir).expect("provision");
        TempArchive { dir }
    }

    /// Import raw mbox text as a new source; returns the archived-email count.
    pub fn import_mbox_str(&self, mbox: &str) -> usize {
        let p = self.dir.join(format!("in-{}.mbox", uniq()));
        std::fs::write(&p, mbox).unwrap();
        pea_engine::ingest::import_mbox(&self.dir, &p, None)
            .expect("import_mbox")
            .archived
    }

    /// Like `import_mbox_str` but returns the full Result (for error-path tests).
    pub fn try_import_mbox_str(&self, mbox: &str) -> Result<usize, String> {
        let p = self.dir.join(format!("in-{}.mbox", uniq()));
        std::fs::write(&p, mbox).unwrap();
        pea_engine::ingest::import_mbox(&self.dir, &p, None).map(|s| s.archived)
    }

    pub fn state(&self, read_only: bool) -> pea_engine::state::AppState {
        pea_engine::state_for_dir(&self.dir, read_only).expect("state")
    }

    /// A writable router (the app's real API surface).
    pub fn router(&self) -> Router {
        pea_engine::api::router(self.state(false))
    }

    /// Convenience: run a closure with a pooled (writable) DB connection.
    pub fn with_conn<T>(&self, f: impl FnOnce(&rusqlite::Connection) -> T) -> T {
        let state = self.state(false);
        let conn = state.pool.get().unwrap();
        f(&conn)
    }
}

impl Drop for TempArchive {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

// ---- HTTP helpers (drive the router directly) ----

pub async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Vec<u8>) {
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

pub async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    let (s, b) = send(app, "GET", uri, None).await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

pub async fn post_json(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let (s, b) = send(app, "POST", uri, Some(body)).await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

pub async fn put_json(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let (s, b) = send(app, "PUT", uri, Some(body)).await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

// ---- fixture builders ----

/// One mbox message block (RFC5322). `extra_headers` are inserted verbatim
/// (e.g. "Content-Type: ...", "References: ...", "In-Reply-To: ..."). The
/// leading "From " separator is required by mbox parsers to split messages.
pub fn mbox_msg(
    message_id: &str,
    from: &str,
    to: &str,
    subject: &str,
    extra_headers: &[&str],
    body: &str,
) -> String {
    let mut s = String::new();
    s.push_str("From MAILER-DAEMON Mon Jan  1 00:00:00 2024\n");
    s.push_str(&format!("Message-ID: {message_id}\n"));
    s.push_str(&format!("From: {from}\n"));
    s.push_str(&format!("To: {to}\n"));
    s.push_str(&format!("Subject: {subject}\n"));
    s.push_str("Date: Mon, 01 Jan 2024 00:00:00 +0000\n");
    for h in extra_headers {
        s.push_str(h);
        s.push('\n');
    }
    s.push('\n');
    s.push_str(body);
    if !body.ends_with('\n') {
        s.push('\n');
    }
    s.push('\n');
    s
}

/// multipart/mixed message with one attachment part (body given verbatim).
pub fn mbox_with_attachment(
    mid: &str,
    subject: &str,
    body: &str,
    filename: &str,
    content_type: &str,
    att: &str,
) -> String {
    let ct = r#"Content-Type: multipart/mixed; boundary="XB""#;
    let mime = format!(
        "--XB\nContent-Type: text/plain; charset=utf-8\n\n{body}\n--XB\n\
         Content-Type: {content_type}; name=\"{filename}\"\n\
         Content-Disposition: attachment; filename=\"{filename}\"\n\n{att}\n--XB--\n"
    );
    mbox_msg(mid, "Alice <alice@example.com>", "bob@example.com", subject, &[ct], &mime)
}

/// A single plain-text email as a full mbox (one message).
pub fn simple_mbox(subject: &str, body: &str) -> String {
    mbox_msg(
        &format!("<{}@example.com>", uniq()),
        "Alice <alice@example.com>",
        "Bob <bob@example.com>",
        subject,
        &[],
        body,
    )
}
