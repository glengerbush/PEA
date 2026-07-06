//! Regression tests for bugs found by the correctness audit. Each asserts the
//! CORRECT (spec) behavior — they failed before the fix.
mod common;
use common::*;

async fn all_ids(app: &axum::Router) -> Vec<String> {
    let (_, b) = get_json(app, "/api/v1/archived-emails?limit=50").await;
    b["hits"].as_array().unwrap().iter().map(|h| h["id"].as_str().unwrap().to_string()).collect()
}

// BUG (data loss): approving a duplicate group whose keeper does not exist must
// NOT delete the "duplicate" copies — that would destroy the last copies.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_exact_preserves_data_when_keeper_missing() {
    let a = TempArchive::new();
    a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<k1@x>", "Alice <a@x.com>", "b@x.com", "K1", &[], "x"),
        mbox_msg("<k2@x>", "Alice <a@x.com>", "b@x.com", "K2", &[], "x"),
    ));
    let app = a.router();
    let ids = all_ids(&app).await;
    assert_eq!(ids.len(), 2);

    // keeper does not exist → must be a no-op
    let (s, res) = post_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact/approve",
        json!({"groups":[{"groupKey":"g","keeperEmailId":"nonexistent","duplicateEmailIds":[ids[0], ids[1]]}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedEmails"], json!(0), "nothing deleted when keeper absent");
    let (_, after) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(after["total"], json!(2), "both emails preserved");

    // with a real keeper: the duplicate is deleted, keeper survives
    let (s, res) = post_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact/approve",
        json!({"groups":[{"groupKey":"g","keeperEmailId":ids[0],"duplicateEmailIds":[ids[1]]}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedEmails"], json!(1));
    let (_, after) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(after["total"], json!(1));
    assert_eq!(after["hits"][0]["id"], json!(ids[0]), "keeper survived");
}

// BUG: after a tag edit, the tag must become full-text searchable — the FTS
// `meta` column must be re-indexed (the recompute selected a dropped column and
// swallowed the error, so it silently never updated).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tagging_reindexes_fts_meta() {
    let a = TempArchive::new();
    a.import_mbox_str(&mbox_msg("<m@x>", "Alice <a@x.com>", "b@x.com", "Sub", &[], "plainbody"));
    let app = a.router();
    let id = all_ids(&app).await[0].clone();

    let (_, before) = get_json(&app, "/api/v1/archived-emails?q=zebratag").await;
    assert_eq!(before["total"], json!(0), "tag term absent before tagging");

    let (s, _) = post_json(&app, "/api/v1/archived-emails/bulk/tags",
        json!({"emailIds":[id], "addTags":["zebratag"]})).await;
    assert_eq!(s, StatusCode::OK);

    let (_, after) = get_json(&app, "/api/v1/archived-emails?q=zebratag").await;
    assert_eq!(after["total"], json!(1), "tag is full-text searchable after re-index");
}

// BUG (data corruption): two distinct Message-IDs that sanitize to the same
// filename ("<a/b@h>" and "<a_b@h>") must each keep their own stored .eml — the
// second must not overwrite the first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn colliding_message_ids_keep_separate_storage() {
    let a = TempArchive::new();
    let n = a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<a/b@h>", "Alice <a@x.com>", "b@x.com", "SubA", &[], "AAAcontent"),
        mbox_msg("<a_b@h>", "Alice <a@x.com>", "b@x.com", "SubB", &[], "BBBcontent"),
    ));
    assert_eq!(n, 2, "distinct Message-IDs both archived");
    let app = a.router();
    let (_, list) = get_json(&app, "/api/v1/archived-emails").await;
    for h in list["hits"].as_array().unwrap() {
        let id = h["id"].as_str().unwrap();
        let subj = h["subject"].as_str().unwrap();
        let (s, bytes) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/eml"), None).await;
        assert_eq!(s, StatusCode::OK);
        let eml = String::from_utf8_lossy(&bytes);
        let expected = if subj == "SubA" { "AAAcontent" } else { "BBBcontent" };
        assert!(eml.contains(expected), "{subj}'s .eml must contain its own body, not the other's");
    }
}

// BUG (import abort): a malformed PDF attachment must not abort the whole import
// (pdf_extract can panic). The email should still be archived.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_pdf_attachment_does_not_abort_import() {
    let a = TempArchive::new();
    let garbage = "%PDF-1.4 broken xref 9 0 obj trailer <<>> not-a-real-pdf endobj";
    let n = a.try_import_mbox_str(&mbox_with_attachment(
        "<pdf@x>", "HasPdf", "cover text", "doc.pdf", "application/pdf", garbage,
    ));
    assert_eq!(n, Ok(1), "import completes despite the bad PDF");
}

// BUG (silent no-op): ISO-8601 date-range filters must actually constrain the
// query (to_timestamp previously returned None for any non-integer string).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iso_date_range_filters_apply() {
    let a = TempArchive::new();
    // mbox_msg dates every message 2024-01-01T00:00:00Z.
    a.import_mbox_str(&mbox_msg("<dt@x>", "Alice <a@x.com>", "b@x.com", "Dated", &[], "datebody"));
    let app = a.router();

    let (_, b) = get_json(&app, "/api/v1/archived-emails?sentAfter=2023-12-01").await;
    assert_eq!(b["total"], json!(1), "email is after 2023-12-01");
    let (_, b) = get_json(&app, "/api/v1/archived-emails?sentAfter=2024-06-01").await;
    assert_eq!(b["total"], json!(0), "ISO sentAfter actually excludes the earlier email");
    let (_, b) = get_json(&app, "/api/v1/archived-emails?sentBefore=2024-06-01T00:00:00Z").await;
    assert_eq!(b["total"], json!(1), "ISO datetime sentBefore includes it");
}

// BUG: CSV display-name detection must not grab "First Name" as the whole name,
// and normalize_email must strip "mailto:" then re-trim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contacts_csv_name_and_mailto() {
    let a = TempArchive::new();
    let app = a.router();
    let csv = "Title,First Name,Last Name,E-mail Address\n\
               Dr,Ada,Lovelace,ada@x.com\n\
               Mr,Bob,Jones,mailto: bob@x.com\n";
    let (s, res) = post_json(&app, "/api/v1/contacts/import", json!({ "format": "csv", "content": csv })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["imported"], json!(2), "both rows imported (mailto: normalized)");
    let (_, map) = get_json(&app, "/api/v1/contacts/map").await;
    assert_eq!(map["ada@x.com"], json!("Ada Lovelace"), "First + Last, not just First");
    assert_eq!(map["bob@x.com"], json!("Bob Jones"), "mailto: address normalized");
}

// BUG (re-audit #3): a provided-but-unknown ingestionSourceId must scope to
// nothing, not silently fall through to "no filter" (returning every email).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_source_filter_returns_empty() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body"));
    let app = a.router();
    let (_, all) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(all["total"], json!(1));
    let (_, none) = get_json(&app, "/api/v1/archived-emails?ingestionSourceId=does-not-exist").await;
    assert_eq!(none["total"], json!(0), "unknown source → 0 results, not all");
}

// BUG (re-audit #4/#5/#6): ISO date filters with a timezone offset must still
// parse and apply; adversarial page/year values must not panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iso_offset_and_adversarial_values_are_safe() {
    let a = TempArchive::new();
    a.import_mbox_str(&mbox_msg("<d@x>", "A <a@x.com>", "b@x.com", "D", &[], "body")); // sent 2024-01-01
    let app = a.router();
    // negative-offset ISO must be parsed (offset stripped) and the filter applied:
    // a June cutoff excludes the January email — before the fix the '-08:00' broke
    // parsing and the filter was silently ignored (would return 1).
    let (_, b) = get_json(&app, "/api/v1/archived-emails?sentAfter=2024-06-01T00:00:00-08:00").await;
    assert_eq!(b["total"], json!(0), "negative-offset ISO date filter applies");
    // absurd year must not overflow/panic
    let (s, _) = get_json(&app, "/api/v1/archived-emails?sentAfter=300000000-01-01").await;
    assert_eq!(s, StatusCode::OK, "absurd year handled");
    // huge page on the remote-content-issues endpoint must not overflow
    let (s, _) = get_json(&app, "/api/v1/dashboard/remote-content-issues?page=9223372036854775807").await;
    assert_eq!(s, StatusCode::OK, "huge page does not overflow");
}

// BUG (HIGH): a Gmail message with multiple labels in one X-Gmail-Labels header
// must be findable under EVERY label, not just the first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gmail_labels_all_become_tags() {
    let a = TempArchive::new();
    a.import_mbox_str(&mbox_msg(
        "<g@x>", "Alice <a@x.com>", "b@x.com", "Labeled",
        &["X-Gmail-Labels: Inbox,Work,Important"], "labelbody",
    ));
    let app = a.router();
    for tag in ["Inbox", "Work", "Important"] {
        let (_, b) = get_json(&app, &format!("/api/v1/archived-emails?tags={tag}")).await;
        assert_eq!(b["total"], json!(1), "message must be findable under tag '{tag}'");
    }
    let (_, facets) = get_json(&app, "/api/v1/archived-emails/facets").await;
    let tags: Vec<String> = facets["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
    for want in ["Important", "Inbox", "Work"] {
        assert!(tags.contains(&want.to_string()), "facets must list '{want}', got {tags:?}");
    }
}

// BUG (data loss regression guard): the mailbox bulk "Delete" is a SOFT delete —
// the emails move to the trash and stay recoverable until purged there. Existing
// bulk-delete tests only assert the rows leave the default list, which would keep
// passing even if the endpoint were switched back to a permanent delete. Pin the
// recoverable-in-trash + restorable semantics so that regression can't slip by.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_delete_is_recoverable_from_trash() {
    let a = TempArchive::new();
    a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<sd1@x>", "A <a@x.com>", "b@x.com", "Keep me", &[], "one"),
        mbox_msg("<sd2@x>", "A <a@x.com>", "b@x.com", "Trash me", &[], "two"),
    ));
    let app = a.router();
    let ids = all_ids(&app).await;
    assert_eq!(ids.len(), 2);

    let (s, res) = post_json(
        &app,
        "/api/v1/archived-emails/bulk/delete",
        json!({ "emailIds": [ids[0]] }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedCount"], json!(1));

    // Gone from the default mailbox list…
    let (_, list) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(list["total"], json!(1), "removed from the default list");
    // …but recoverable in the trash (soft delete, not a permanent purge).
    let (_, trash) = get_json(&app, "/api/v1/archived-emails?trashed=true").await;
    assert_eq!(trash["total"], json!(1), "bulk delete moves to the trash, not permanent");
    assert_eq!(trash["hits"][0]["id"], json!(ids[0]));

    // …and restorable back into the list.
    let (s, res) = post_json(
        &app,
        "/api/v1/archived-emails/trash/restore",
        json!({ "emailIds": [ids[0]] }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["restoredCount"], json!(1));
    let (_, list) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(list["total"], json!(2), "restored back into the mailbox");
}

// BUG (data loss regression guard): approving an exact-duplicate group sends the
// duplicate copies to the TRASH (recoverable), not a permanent delete, and leaves
// the keeper untouched. The existing approve tests only check "one copy remains",
// which a permanent delete would also satisfy — so assert the approved duplicate
// is still recoverable in the trash.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_exact_duplicates_are_recoverable_from_trash() {
    let a = TempArchive::new();
    let email = mbox_msg("<dupr@x>", "Alice <a@x.com>", "b@x.com", "Dup", &[], "the body");
    a.import_mbox_str(&email);
    a.import_mbox_str(&email);
    let app = a.router();

    let (_, groups) = get_json(&app, "/api/v1/archived-emails/duplicates/exact").await;
    let group = &groups["groups"][0];
    let keeper = group["keeperEmailId"].as_str().unwrap().to_string();
    let dups: Vec<String> = group["emails"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .filter(|id| *id != keeper)
        .collect();
    assert_eq!(dups.len(), 1);

    let (s, res) = post_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact/approve",
        json!({"groups":[{"groupKey": group["groupKey"], "keeperEmailId": keeper, "duplicateEmailIds": dups}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedEmails"], json!(1));

    // Keeper survives in the mailbox; the duplicate leaves the default list…
    let (_, list) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(list["total"], json!(1));
    assert_eq!(list["hits"][0]["id"], json!(keeper), "keeper survives");
    // …but the approved duplicate is recoverable in the trash, not purged.
    let (_, trash) = get_json(&app, "/api/v1/archived-emails?trashed=true").await;
    assert_eq!(trash["total"], json!(1), "approved duplicate went to the trash");
    assert_eq!(trash["hits"][0]["id"], json!(dups[0]));
}

// After the tracking filter learned to recognize a tracker (e.g. Amazon's
// gp/r.html open-redirect), the startup sweep must clear the stale failed/blocked
// asset it recorded before the filter existed — but must NOT remove a genuine
// failed asset the user might still want to retry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sweep_clears_stale_tracking_assets_only() {
    let a = TempArchive::new();
    a.import_mbox_str(&mbox_msg("<sw@x>", "A <a@x.com>", "b@x.com", "Sweep", &[], "body"));
    let app = a.router();
    let id = {
        let (_, b) = get_json(&app, "/api/v1/archived-emails").await;
        b["hits"][0]["id"].as_str().unwrap().to_string()
    };

    a.with_conn(|conn| {
        // A tracker recorded as 'failed' before the filter existed…
        conn.execute(
            "INSERT INTO remote_content_assets (id, email_id, original_url, url_hash, status) \
             VALUES (?,?,?,?,'failed')",
            ["trk", id.as_str(), "http://www.amazon.com/gp/r.html?U=http%3A%2F%2Fx%2Ftransp.gif", "h1"],
        )
        .unwrap();
        // …and a genuine failed image that must be preserved for retry.
        conn.execute(
            "INSERT INTO remote_content_assets (id, email_id, original_url, url_hash, status) \
             VALUES (?,?,?,?,'failed')",
            ["real", id.as_str(), "https://cdn.example.com/hero.png", "h2"],
        )
        .unwrap();
    });

    pea_engine::remote_content::sweep_tracking_assets(&a.state(false));

    a.with_conn(|conn| {
        let trk: i64 = conn
            .query_row("SELECT count(*) FROM remote_content_assets WHERE id='trk'", [], |r| r.get(0))
            .unwrap();
        let real: i64 = conn
            .query_row("SELECT count(*) FROM remote_content_assets WHERE id='real'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trk, 0, "stale tracker asset swept");
        assert_eq!(real, 1, "genuine failed asset preserved");
    });
}
