//! End-to-end import smoke test: provisions a fresh data dir, imports the
//! golden fixture mbox through the real queue pipeline (initial-import →
//! process-mailbox → index-email-batch → sync-cycle-finished), and asserts
//! the archive contents. This inherits the assertions the Node-parity
//! golden harnesses covered before the Node engine was retired.

use std::path::PathBuf;

#[test]
fn imports_the_golden_mbox() {
    let tmp = std::env::temp_dir().join(format!("pea-import-smoke-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    std::fs::create_dir_all(&tmp).unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fixtures/golden.mbox");

    pea_engine::provision::provision(&tmp).expect("provision");
    let stats = pea_engine::ingest::import_mbox(&tmp, &fixture, None).expect("import");
    // 8 messages in the fixture; one is a duplicate message-id and is skipped.
    assert_eq!(stats.archived, 7, "archived email count");

    let state = pea_engine::state_for_dir(&tmp, true).expect("state");
    let conn = state.pool.get().unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };

    assert_eq!(count("SELECT count(*) FROM archived_emails"), 7);
    // notes.txt (regular) + logo.png (inline cid) — sha-deduplicated store.
    assert_eq!(count("SELECT count(*) FROM attachments"), 2);
    assert_eq!(count("SELECT count(*) FROM email_attachments"), 2);
    // Every email indexed, and FTS behaves (2 reprap hits incl. the reply).
    assert_eq!(count("SELECT count(*) FROM email_fts"), 7);
    assert_eq!(
        count("SELECT count(*) FROM email_fts WHERE email_fts MATCH 'reprap'"),
        2
    );
    assert_eq!(
        count("SELECT count(*) FROM email_fts WHERE email_fts MATCH 'esteps'"),
        1,
        "attachment text is extracted and searchable"
    );
    // Duplicate fingerprints are populated (subject hash on every email).
    assert_eq!(
        count("SELECT count(*) FROM archived_emails WHERE duplicate_subject_hash IS NOT NULL"),
        7
    );
    // Threading: the reply joined msg-001's thread.
    assert_eq!(
        count(
            "SELECT count(*) FROM archived_emails WHERE thread_id = '<msg-001@example.com>'"
        ),
        2
    );
    // The source finished in the imported state.
    let (status, message): (String, String) = conn
        .query_row(
            "SELECT status, last_sync_status_message FROM ingestion_sources",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "imported");
    assert_eq!(message, "Import finished. Archived 1 mailbox(es).");
    // Stored emails are encrypted at rest (storage key was provisioned).
    let storage_path: String = conn
        .query_row("SELECT storage_path FROM archived_emails LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let raw = std::fs::read(state.storage_root().join(&storage_path)).unwrap();
    assert!(raw.starts_with(b"oa_enc_idf_v1::"), "storage is encrypted");

    std::fs::remove_dir_all(&tmp).ok();
}
