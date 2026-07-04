//! Coverage for the synchronous job-runtime paths (queue drain → processors →
//! remote-content archive) and queue recovery, driven directly (no async loop).
mod common;
use common::*;
use pea_engine::{queue, remote_content};

fn html_email(remote_url: &str) -> String {
    let mime = "Content-Type: text/html; charset=utf-8";
    let body = format!("<html><body><img src=\"{remote_url}\"><p>hi</p></body></html>");
    mbox_msg("<rc@x>", "Alice <a@x.com>", "b@x.com", "RemoteImg", &[mime], &body)
}

#[test]
fn remote_content_archive_blocks_loopback_urls() {
    let a = TempArchive::new();
    a.import_mbox_str(&html_email("http://127.0.0.1:1/logo.png"));
    let state = a.state(false);
    let id: String = state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT id FROM archived_emails LIMIT 1", [], |r| r.get(0))
        .unwrap();

    // enqueue archiving, then run the whole job pipeline synchronously
    remote_content::enqueue_archive(&state, &[id.clone()]);
    queue::drain_for_cli(&state).unwrap();

    // The SSRF guard blocks the loopback URL → the asset reaches a terminal state.
    let (assets, blocked): (i64, i64) = state
        .pool
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*), count(*) FILTER (WHERE status = 'blocked') \
             FROM remote_content_assets WHERE email_id = ?",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(assets, 1, "one remote asset tracked");
    assert_eq!(blocked, 1, "loopback URL blocked by the SSRF guard");

    let status: String = state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT remote_content_status FROM archived_emails WHERE id = ?", [&id], |r| r.get(0))
        .unwrap();
    assert_ne!(status, "pending", "email reached a terminal remote-content status");
}

#[test]
fn recover_interrupted_reclassifies_active_jobs() {
    let a = TempArchive::new();
    let state = a.state(false);
    queue::send_job(&state, "ingestion", "noop", &serde_json::json!({}), queue::no_retry());
    let conn = state.pool.get().unwrap();

    // an interrupted (active) job still under its attempt budget → retried
    conn.execute("UPDATE jobs SET state = 'active', attempts = 0, max_attempts = 3", []).unwrap();
    queue::recover_interrupted(&conn);
    let s: String = conn.query_row("SELECT state FROM jobs LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(s, "pending", "under max attempts → back to pending");

    // one that has exhausted its attempts → failed
    conn.execute("UPDATE jobs SET state = 'active', attempts = 3, max_attempts = 3", []).unwrap();
    queue::recover_interrupted(&conn);
    let s: String = conn.query_row("SELECT state FROM jobs LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(s, "failed", "at max attempts → failed");
}

#[test]
fn session_lifecycle_and_stale_cleanup() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body")); // creates an ingestion source
    let state = a.state(false);
    let conn = state.pool.get().unwrap();
    let src: String = conn
        .query_row("SELECT id FROM ingestion_sources LIMIT 1", [], |r| r.get(0))
        .unwrap();

    let sid = pea_engine::sessions::create(&conn, &src, 2, true).unwrap();
    pea_engine::sessions::heartbeat(&conn, &sid);
    let r1 = pea_engine::sessions::record_mailbox_result(&conn, &sid, Ok(&serde_json::json!({}))).unwrap();
    assert!(!r1.is_last, "1 of 2 processed");
    let r2 = pea_engine::sessions::record_mailbox_result(&conn, &sid, Err("boom")).unwrap();
    assert!(r2.is_last, "2 of 2 processed → last");

    let (c, f): (i64, i64) = conn
        .query_row("SELECT completed_mailboxes, failed_mailboxes FROM import_sessions WHERE id = ?", [&sid], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((c, f), (1, 1));
    pea_engine::sessions::finalize(&conn, &sid);
    pea_engine::sessions::clean_stale_sessions(&conn);
    // find_by_id error path
    assert!(pea_engine::sessions::find_by_id(&conn, "no-such-session").is_err());
}

#[test]
fn unknown_job_is_marked_failed_not_crash() {
    let a = TempArchive::new();
    let state = a.state(false);
    queue::send_job(&state, "ingestion", "totally-unknown-job", &serde_json::json!({}), queue::no_retry());
    queue::drain_for_cli(&state).unwrap();
    let s: String = state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT state FROM jobs LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(s, "failed", "unknown job type → failed; the pipeline survives");
}

#[test]
fn reimport_is_idempotent() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "reimportbody"));
    let state = a.state(false);
    let count = |s: &pea_engine::state::AppState| -> i64 {
        s.pool.get().unwrap().query_row("SELECT count(*) FROM archived_emails", [], |r| r.get(0)).unwrap()
    };
    let before = count(&state);
    let src: String = state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT id FROM ingestion_sources LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let conn = state.pool.get().unwrap();
    pea_engine::sources::trigger_reimport(&state, &conn, &src).unwrap();
    drop(conn);
    queue::drain_for_cli(&state).unwrap();
    assert_eq!(before, count(&state), "re-sync dedups by message-id; no duplicate emails");
}

#[test]
fn singleton_jobs_are_suppressed() {
    let a = TempArchive::new();
    let state = a.state(false);
    let first = queue::send_job(&state, "ingestion", "x", &serde_json::json!({}), queue::master_job_options("only-one"));
    let second = queue::send_job(&state, "ingestion", "x", &serde_json::json!({}), queue::master_job_options("only-one"));
    assert!(first.is_some(), "first enqueued");
    assert!(second.is_none(), "duplicate singleton suppressed");
}

#[test]
fn remove_jobs_by_source_id_clears_queue() {
    let a = TempArchive::new();
    let state = a.state(false);
    queue::send_job(
        &state,
        "ingestion",
        "process",
        &serde_json::json!({ "ingestionSourceId": "src-1" }),
        queue::no_retry(),
    );
    let conn = state.pool.get().unwrap();
    let removed = queue::remove_jobs_by_source_id(&conn, "src-1");
    assert_eq!(removed, 1, "the pending job for the source is removed");
}
