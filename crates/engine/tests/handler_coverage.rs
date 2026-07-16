//! Integration coverage for the handler / duplicates / preview / dashboard
//! surfaces that the smoke tests don't reach. Behavior is asserted from the
//! documented contracts.
mod common;
use common::*;

// 1x1 transparent PNG.
const PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

fn preview_email() -> String {
    let mime = r#"Content-Type: multipart/related; boundary="R""#;
    let body = format!(
        "--R\nContent-Type: text/html; charset=utf-8\n\n\
         <html><body><img src=\"cid:Logo\"><script>alert(1)</script><p>hello preview</p></body></html>\n\
         --R\nContent-Type: image/png\nContent-ID: <logo>\nContent-Transfer-Encoding: base64\n\n{PNG_B64}\n--R--\n"
    );
    mbox_msg("<pv@x>", "Alice <a@x.com>", "b@x.com", "PreviewMail", &[mime], &body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_sanitizes_and_inlines_cid() {
    let a = TempArchive::new();
    a.import_mbox_str(&preview_email());
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };

    let (s, bytes) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/preview"), None).await;
    assert_eq!(s, StatusCode::OK);
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("hello preview"), "body text preserved");
    assert!(!html.contains("<script"), "script stripped");
    assert!(!html.contains("alert(1)"), "script body stripped");
    // the inline image must not survive as a live cid: reference
    assert!(!html.contains("cid:Logo"), "cid reference resolved or removed");
    // …and "removed" is not good enough: the image the sender embedded has to
    // actually render. The stored .eml is hollowed, so this only passes when the
    // preview resolves `cid:Logo` against the blob store (matching the part's
    // `Content-ID: <logo>` case-insensitively) and inlines it as a data: URI.
    assert!(html.contains("<img"), "the inline image survives sanitization");
    assert!(html.contains("data:image/png;base64,"), "cid image inlined from the blob store");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_duplicates_list_and_approve() {
    let a = TempArchive::new();
    // Same complete email imported as two separate sources → both archived and
    // classified Exact.
    let email = mbox_msg("<dup@x>", "Alice <a@x.com>", "b@x.com", "Dup", &[], "the body");
    a.import_mbox_str(&email);
    a.import_mbox_str(&email);
    let app = a.router();
    let (_, all) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(all["total"], json!(2), "both copies archived");

    let (s, groups) = get_json(&app, "/api/v1/archived-emails/duplicates/exact").await;
    assert_eq!(s, StatusCode::OK);
    assert!(groups["totalGroups"].as_i64().unwrap() >= 1, "an exact-duplicate group is found");
    let group = &groups["groups"][0];
    assert_eq!(group["classification"], json!("exact"));
    let keeper = group["keeperEmailId"].as_str().unwrap().to_string();
    let dups: Vec<String> = group["emails"].as_array().unwrap().iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .filter(|id| *id != keeper)
        .collect();
    assert_eq!(dups.len(), 1);

    let (s, res) = post_json(&app, "/api/v1/archived-emails/duplicates/exact/approve",
        json!({"groups":[{"groupKey": group["groupKey"], "keeperEmailId": keeper, "duplicateEmailIds": dups}]})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedEmails"], json!(1));
    let (_, after) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(after["total"], json!(1), "one copy remains");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_only_match_is_not_an_exact_group() {
    let a = TempArchive::new();
    // Same body but different Message-ID and recipients must not form even a
    // Likely group. This is the forwarded-to-someone-else safeguard.
    a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<fz1@x>", "Alice <alice@x.com>", "b@x.com", "Weekly Meeting", &[], "identical agenda body"),
        mbox_msg("<fz2@x>", "Alice <alice@x.com>", "c@x.com", "Weekly Meeting", &[], "identical agenda body"),
    ));
    let app = a.router();

    let (s, groups) = get_json(&app, "/api/v1/archived-emails/duplicates/exact").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(groups["totalGroups"].as_i64().unwrap(), 0, "body alone must not group");
    assert_eq!(groups["classificationCounts"]["likely"].as_i64().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn likely_requires_the_same_semantic_message() {
    let a = TempArchive::new();
    // Everything semantic is identical, but provider Message-IDs differ. This
    // is the narrow, review-only Likely case.
    a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<likely-1@x>", "Alice <alice@x.com>", "bob@x.com", "Receipt", &[], "same receipt body"),
        mbox_msg("<likely-2@x>", "Alice <alice@x.com>", "bob@x.com", "Receipt", &[], "same receipt body"),
    ));
    let app = a.router();

    let (s, groups) = get_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact?classification=likely",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(groups["totalGroups"], json!(1));
    assert_eq!(groups["groups"][0]["classification"], json!("likely"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachment_filename_difference_requires_review() {
    let a = TempArchive::new();
    // The attachment bytes are identical, but its filename is not. Import via
    // separate sources so storage deduplication does not collapse the metadata.
    a.import_mbox_str(&mbox_with_attachment(
        "<attachment-name-1@x>",
        "Report",
        "same body",
        "report.txt",
        "text/plain",
        "same attachment",
    ));
    a.import_mbox_str(&mbox_with_attachment(
        "<attachment-name-2@x>",
        "Report",
        "same body",
        "renamed.txt",
        "text/plain",
        "same attachment",
    ));
    let app = a.router();

    let (_, exact) = get_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact?classification=exact",
    )
    .await;
    assert_eq!(exact["totalGroups"], json!(0));
    let (_, likely) = get_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact?classification=likely",
    )
    .await;
    assert_eq!(likely["totalGroups"], json!(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn likely_rejects_reply_chains_and_forwards() {
    let a = TempArchive::new();
    // Same sender, recipients, subject and even exact timestamp, but a reply
    // adds one line. Body equality is mandatory, so rapid replies cannot match.
    let rapid_replies = format!(
        "{}{}",
        mbox_msg("<reply-1@x>", "Alice <alice@x.com>", "bob@x.com", "Re: Project", &[], "Original line"),
        mbox_msg("<reply-2@x>", "Alice <alice@x.com>", "bob@x.com", "Re: Project", &[], "Original line\nOne new line"),
    );
    // Same content sent to another person. Recipient equality is mandatory, so
    // a forwarded/re-addressed copy cannot match.
    let forwards = format!(
        "{}{}",
        mbox_msg("<forward-1@x>", "Alice <alice@x.com>", "bob@x.com", "FYI", &[], "shared content"),
        mbox_msg("<forward-2@x>", "Alice <alice@x.com>", "carol@x.com", "FYI", &[], "shared content"),
    );
    // A normal back-and-forth changes sender/recipient direction and body.
    let conversation = format!(
        "{}{}",
        mbox_msg("<turn-1@x>", "Alice <alice@x.com>", "bob@x.com", "Status", &[], "Ready?"),
        mbox_msg("<turn-2@x>", "Bob <bob@x.com>", "alice@x.com", "Status", &[], "Ready?\nYes."),
    );
    a.import_mbox_str(&format!("{rapid_replies}{forwards}{conversation}"));
    let app = a.router();

    let (_, groups) = get_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact?classification=likely",
    )
    .await;
    assert_eq!(groups["totalGroups"], json!(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_delete_never_touches_likely_groups() {
    let a = TempArchive::new();
    let exact = mbox_msg(
        "<bulk-exact@x>",
        "Alice <alice@x.com>",
        "bob@x.com",
        "Exact",
        &[],
        "exact body",
    );
    a.import_mbox_str(&exact);
    a.import_mbox_str(&exact);
    a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<bulk-likely-1@x>", "Alice <alice@x.com>", "bob@x.com", "Likely", &[], "likely body"),
        mbox_msg("<bulk-likely-2@x>", "Alice <alice@x.com>", "bob@x.com", "Likely", &[], "likely body"),
    ));
    let app = a.router();

    let (status, result) = post_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact/approve-all",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["deletedEmails"], json!(1), "only the Exact copy is trashed");

    let (_, likely) = get_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact?classification=likely",
    )
    .await;
    assert_eq!(likely["totalGroups"], json!(1), "Likely group remains for review");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_duplicate_ignore_hides_group() {
    let a = TempArchive::new();
    // Same Message-ID imported twice → one exact-duplicate group.
    let email = mbox_msg("<ig@x>", "Bob <bob@x.com>", "b@x.com", "Status Report", &[], "same body text");
    a.import_mbox_str(&email);
    a.import_mbox_str(&email);
    let app = a.router();

    let (_, groups) = get_json(&app, "/api/v1/archived-emails/duplicates/exact").await;
    assert_eq!(groups["totalGroups"].as_i64().unwrap(), 1);
    let gkey = groups["groups"][0]["groupKey"].as_str().unwrap().to_string();

    // empty ignore is a well-formed no-op
    let (s, res) = post_json(&app, "/api/v1/archived-emails/duplicates/exact/ignore", json!({ "groupKeys": [] })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["ignoredGroups"].as_i64().unwrap(), 0);

    // ignoring the key records it and drops the group from the listing
    let (s, res) = post_json(&app, "/api/v1/archived-emails/duplicates/exact/ignore", json!({ "groupKeys": [gkey] })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["ignoredGroups"].as_i64().unwrap(), 1);
    let (_, after) = get_json(&app, "/api/v1/archived-emails/duplicates/exact").await;
    assert_eq!(after["totalGroups"].as_i64().unwrap(), 0, "ignored group is hidden");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_trashes_and_restore_brings_back() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Trashable", "body"));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };

    // delete → soft delete: gone from the default list, present in the trash view
    let (s, _) = send(&app, "DELETE", &format!("/api/v1/archived-emails/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, list) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(list["total"], json!(0), "hidden from the default list");
    let (_, trash) = get_json(&app, "/api/v1/archived-emails?trashed=true").await;
    assert_eq!(trash["total"], json!(1), "shown in the trash");

    // restore → back in the default list
    let (s, res) = post_json(&app, "/api/v1/archived-emails/trash/restore", json!({"emailIds":[id]})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["restoredCount"], json!(1));
    let (_, list) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(list["total"], json!(1), "restored to the list");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_trash_preserves_attachment_shared_with_remaining_email() {
    let a = TempArchive::new();
    let mime = r#"Content-Type: multipart/mixed; boundary="B""#;
    let parts = "--B\nContent-Type: text/plain\n\nbody\n\
        --B\nContent-Type: text/plain; name=\"shared.txt\"\nContent-Disposition: attachment; filename=\"shared.txt\"\n\nSHAREDDATA\n--B--\n";
    // Two messages in the SAME source carrying the identical attachment → one
    // deduped `attachments` row referenced by both emails.
    a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<m1@x>", "Alice <a@x.com>", "b@x.com", "One", &[mime], parts),
        mbox_msg("<m2@x>", "Alice <a@x.com>", "b@x.com", "Two", &[mime], parts),
    ));
    let app = a.router();

    let (_, all) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(all["total"], json!(2), "both archived");
    let ids: Vec<String> = all["hits"].as_array().unwrap().iter()
        .map(|h| h["id"].as_str().unwrap().to_string()).collect();
    let att_count = || a.with_conn(|c| c.query_row("SELECT count(*) FROM attachments", [], |r| r.get::<_, i64>(0)).unwrap());
    assert_eq!(att_count(), 1, "attachment deduped to one row");

    // Trash the first, empty the trash → attachment survives (still used by the second).
    let (s, _) = send(&app, "DELETE", &format!("/api/v1/archived-emails/{}", ids[0]), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, res) = post_json(&app, "/api/v1/archived-emails/trash/empty", json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedCount"], json!(1));
    assert_eq!(att_count(), 1, "shared attachment preserved while another email uses it");

    // Trash the second, empty the trash → nothing references it now, so it's GC'd.
    let (s, _) = send(&app, "DELETE", &format!("/api/v1/archived-emails/{}", ids[1]), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = post_json(&app, "/api/v1/archived-emails/trash/empty", json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(att_count(), 0, "attachment removed once no email references it");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachments_archive_zip_has_unique_entries() {
    let a = TempArchive::new();
    // three distinct attachments, two with the same basename plus a real "a-2.txt"
    let mime = r#"Content-Type: multipart/mixed; boundary="B""#;
    let parts = "--B\nContent-Type: text/plain\n\ncover\n\
        --B\nContent-Type: text/plain; name=\"a.txt\"\nContent-Disposition: attachment; filename=\"a.txt\"\n\nAAA\n\
        --B\nContent-Type: text/plain; name=\"a.txt\"\nContent-Disposition: attachment; filename=\"a.txt\"\n\nBBB\n\
        --B\nContent-Type: text/plain; name=\"a-2.txt\"\nContent-Disposition: attachment; filename=\"a-2.txt\"\n\nCCC\n--B--\n";
    a.import_mbox_str(&mbox_msg("<z@x>", "Alice <a@x.com>", "b@x.com", "Zips", &[mime], parts));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };

    let (s, bytes) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/attachments/archive"), None).await;
    assert_eq!(s, StatusCode::OK);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().to_string()).collect();
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(names.len(), 3, "three attachments");
    assert_eq!(unique.len(), 3, "all zip entry names unique (regression #10), got {names:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_reimport_and_unmerge() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("A", "body a"));
    a.import_mbox_str(&simple_mbox("B", "body b"));
    let app = a.router();
    let (_, sources) = get_json(&app, "/api/v1/ingestion-sources").await;
    let s1 = sources[0]["id"].as_str().unwrap().to_string();
    let s2 = sources[1]["id"].as_str().unwrap().to_string();

    // merge s2 into s1
    a.with_conn(|c| {
        c.execute(
            "UPDATE ingestion_sources SET merged_into_id = ?1 WHERE id = ?2",
            [s1.as_str(), s2.as_str()],
        )
        .unwrap();
    });

    // force-sync the root (enqueues, returns 202)
    let (s, _) = post_json(&app, &format!("/api/v1/ingestion-sources/{s1}/reimport"), json!({})).await;
    assert_eq!(s, StatusCode::ACCEPTED);

    // unmerge the child
    let (s, unmerged) = post_json(&app, &format!("/api/v1/ingestion-sources/{s2}/unmerge"), json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(unmerged["mergedIntoId"], json!(null), "child no longer merged");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_source_via_api() {
    let a = TempArchive::new();
    let mbox_path = a.dir.join("real.mbox");
    std::fs::write(&mbox_path, simple_mbox("Created", "createdbody")).unwrap();
    let app = a.router();

    // valid mbox source is created
    let (s, created) = post_json(
        &app,
        "/api/v1/ingestion-sources",
        json!({"name":"MySource","provider":"mbox_import","providerConfig":{"localFilePath": mbox_path.to_str().unwrap()}}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(created["provider"], json!("mbox_import"));
    assert_eq!(created["name"], json!("MySource"));

    // an unknown provider is rejected
    let (s, _) = post_json(
        &app,
        "/api/v1/ingestion-sources",
        json!({"name":"Bad","provider":"nope","providerConfig":{}}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writes_edge_cases() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body"));
    let app = a.router();
    // approving an empty group set is a no-op
    let (s, res) = post_json(&app, "/api/v1/archived-emails/duplicates/exact/approve", json!({ "groups": [] })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["approvedGroups"], json!(0));
    // an unsupported contacts format is rejected
    let (s, _) = post_json(&app, "/api/v1/contacts/import", json!({ "format": "xml", "content": "x" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // approve with a group whose keeper is empty → skipped, nothing deleted
    let (s, res) = post_json(
        &app,
        "/api/v1/archived-emails/duplicates/exact/approve",
        json!({"groups":[{"groupKey":"g","keeperEmailId":"","duplicateEmailIds":["x"]}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedEmails"], json!(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachments_archive_404_when_none() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("NoAtt", "body"));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };
    let (s, _) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/attachments/archive"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "no attachments → 404");
    // eml + raw for an unknown id → 404
    let (s, _) = send(&app, "GET", "/api/v1/archived-emails/unknown/eml", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = send(&app, "GET", "/api/v1/archived-emails/unknown/raw", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_sort_and_filter_variations() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "filterbody"));
    let app = a.router();
    // every sort column, both directions
    for sort in ["sentAt", "archivedAt", "sender", "subject", "sizeBytes"] {
        for dir in ["asc", "desc"] {
            let (s, _) = get_json(&app, &format!("/api/v1/archived-emails?sort={sort}&direction={dir}")).await;
            assert_eq!(s, StatusCode::OK, "sort={sort} dir={dir}");
        }
    }
    // each filter arm of build_filter_sql
    for q in [
        "to=x@y.com",
        "cc=x@y.com",
        "bcc=x@y.com",
        "importSource=MyMail",
        "sourcePath=Inbox",
        "attachmentExt=pdf,txt",
        "hasAttachments=true",
        "hasAttachments=false",
        "tags=work",
        "sentAfter=1&sentBefore=9999999999999",
        "archivedAfter=2020-01-01&archivedBefore=2030-01-01",
        "q=filterbody&matchingStrategy=all",
        "ingestionSourceId=nonexistent",
    ] {
        let (s, _) = get_json(&app, &format!("/api/v1/archived-emails?{q}")).await;
        assert_eq!(s, StatusCode::OK, "filter {q}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_endpoints_are_well_formed() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body"));
    let app = a.router();
    for uri in [
        "/api/v1/dashboard/stats",
        "/api/v1/dashboard/ingestion-history",
        "/api/v1/dashboard/ingestion-sources",
        "/api/v1/dashboard/indexed-insights",
        "/api/v1/dashboard/remote-content-issues",
    ] {
        let (s, _) = get_json(&app, uri).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
    }
    let (_, insights) = get_json(&app, "/api/v1/dashboard/indexed-insights").await;
    assert!(insights["topSenders"].is_array());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_content_endpoints() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body"));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };

    let (s, assets) = get_json(&app, &format!("/api/v1/archived-emails/{id}/remote-assets")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(assets.is_array());
    // enqueue archiving for the email (queues a job; no network in the test)
    let (s, _) = post_json(&app, &format!("/api/v1/archived-emails/{id}/remote-content/archive"), json!({})).await;
    assert_eq!(s, StatusCode::ACCEPTED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_download_and_traversal_guard() {
    let a = TempArchive::new();
    a.state(false).storage_put("test/hello.txt", b"hello-bytes").unwrap();
    let app = a.router();
    let (s, bytes) = send(&app, "GET", "/api/v1/storage/download?path=test/hello.txt", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(bytes, b"hello-bytes");
    // path traversal is rejected
    let (s, _) = send(&app, "GET", "/api/v1/storage/download?path=../../../etc/passwd", None).await;
    assert!(s == StatusCode::BAD_REQUEST || s == StatusCode::NOT_FOUND, "traversal blocked");
    // quicklook requires a path
    let (s, _) = post_json(&app, "/api/v1/attachments/quicklook", json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_queue_details_and_vcf_contacts() {
    let a = TempArchive::new();
    let app = a.router();
    let (s, jobs) = get_json(&app, "/api/v1/jobs/queues/ingestion?status=completed&limit=10").await;
    assert_eq!(s, StatusCode::OK);
    assert!(jobs["jobs"].is_array());
    assert!(jobs["pagination"]["totalPages"].as_i64().unwrap() >= 0);
    // negative limit must not disable the LIMIT (regression #19)
    let (s, _) = get_json(&app, "/api/v1/jobs/queues/ingestion?status=completed&limit=-1").await;
    assert_eq!(s, StatusCode::OK);

    // vCard contacts import
    let vcf = "BEGIN:VCARD\nFN:Jane Doe\nEMAIL:jane@x.com\nEND:VCARD\n";
    let (s, res) = post_json(&app, "/api/v1/contacts/import", json!({ "format": "vcf", "content": vcf })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["imported"], json!(1));
    let (_, map) = get_json(&app, "/api/v1/contacts/map").await;
    assert_eq!(map["jane@x.com"], json!("Jane Doe"));
}
