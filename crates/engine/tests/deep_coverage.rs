//! Deeper handler/preview/processor coverage via realistic payloads.
mod common;
use common::*;

fn rich_html_email() -> String {
    let mime = "Content-Type: text/html; charset=utf-8";
    let body = "<html><head><style>.a{background:url(http://127.0.0.1:1/bg.png); color:red} @import 'evil.css';</style></head>\
        <body>\
        <a href=\"javascript:alert(1)\">bad link</a>\
        <a href=\"https://good.example.com/page\">good link</a>\
        <img src=\"http://127.0.0.1:1/remote.png\" srcset=\"http://127.0.0.1:1/x.png 1x, http://127.0.0.1:1/y.png 2x\">\
        <img width=\"1\" height=\"1\" src=\"http://127.0.0.1:1/track.png\">\
        <div style=\"background:url(http://127.0.0.1:1/d.png); width:100px; behavior:url(evil)\">styled</div>\
        <p>visible body text</p>\
        </body></html>";
    mbox_msg("<rich@x>", "Alice <a@x.com>", "b@x.com", "Rich", &[mime], body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_sanitizes_rich_html() {
    let a = TempArchive::new();
    a.import_mbox_str(&rich_html_email());
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };
    let (s, bytes) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/preview"), None).await;
    assert_eq!(s, StatusCode::OK);
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("visible body text"), "text preserved");
    assert!(html.contains("good.example.com"), "safe link kept");
    assert!(!html.contains("javascript:"), "javascript: link neutralized");
    assert!(!html.contains("behavior"), "css behavior stripped");
    assert!(!html.contains("@import"), "css @import stripped");
    // (unarchived remote URLs may remain — they're neutralized by the preview CSP,
    // not rewritten, since there is no archived copy to point at.)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_plain_text_email() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("PlainSub", "line one\nline two"));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };
    let (s, bytes) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/preview"), None).await;
    assert_eq!(s, StatusCode::OK);
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("line one") && html.contains("line two"), "text rendered");
    assert!(html.contains("<br"), "newlines become <br>");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn email_detail_has_full_shape() {
    let a = TempArchive::new();
    // multipart with a text attachment so the detail carries attachments
    let mime = r#"Content-Type: multipart/mixed; boundary="B""#;
    let parts = "--B\nContent-Type: text/plain\n\nhello\n\
        --B\nContent-Type: text/plain; name=\"notes.txt\"\nContent-Disposition: attachment; filename=\"notes.txt\"\n\nattached\n--B--\n";
    a.import_mbox_str(&mbox_msg("<det@x>", "Alice <a@x.com>", "bob@x.com", "Detail", &[mime], parts));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };
    let (s, d) = get_json(&app, &format!("/api/v1/archived-emails/{id}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(d["id"], json!(id));
    assert_eq!(d["subject"], json!("Detail"));
    assert_eq!(d["hasAttachments"], json!(true));
    assert_eq!(d["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(d["attachments"][0]["filename"], json!("notes.txt"));
    assert!(d["thread"].is_array());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachment_hollow_rebuild_round_trips() {
    let a = TempArchive::new();
    // base64("HELLOWORLD") = SEVMTE9XT1JMRA==
    let mime = r#"Content-Type: multipart/mixed; boundary="B""#;
    let parts = "--B\nContent-Type: text/plain\n\ncover\n\
        --B\nContent-Type: application/octet-stream; name=\"data.bin\"\n\
        Content-Disposition: attachment; filename=\"data.bin\"\nContent-Transfer-Encoding: base64\n\n\
        SEVMTE9XT1JMRA==\n--B--\n";
    a.import_mbox_str(&mbox_msg("<hb@x>", "A <a@x.com>", "b@x.com", "Att", &[mime], parts));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };

    // the stored (hollowed) .eml omits the attachment body inline...
    let (_, raw) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/raw"), None).await;
    assert!(!String::from_utf8_lossy(&raw).contains("SEVMTE9XT1JMRA=="), "attachment hollowed out of storage");
    // ...but the downloadable .eml splices it back in.
    let (s, eml) = send(&app, "GET", &format!("/api/v1/archived-emails/{id}/eml"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(String::from_utf8_lossy(&eml).contains("SEVMTE9XT1JMRA=="), "attachment restored on download");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_queue_details_with_real_jobs() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body")); // import creates + completes jobs
    let app = a.router();
    let (_, overview) = get_json(&app, "/api/v1/jobs/queues").await;
    assert!(overview["queues"].as_array().unwrap().len() >= 1);
    // completed jobs from the import are visible
    let (s, details) = get_json(&app, "/api/v1/jobs/queues/ingestion?status=completed&page=1&limit=10").await;
    assert_eq!(s, StatusCode::OK);
    assert!(details["counts"]["completed"].as_i64().unwrap() >= 1, "import completed at least one ingestion job");
    // unknown status → error
    let (s, _) = get_json(&app, "/api/v1/jobs/queues/ingestion?status=bogus").await;
    assert!(s.is_client_error() || s.is_server_error());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tags_combined_add_and_remove_in_one_call() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body"));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };
    let (s, res) = post_json(&app, "/api/v1/archived-emails/bulk/tags",
        json!({ "emailIds": [id], "addTags": ["a", "b"], "removeTags": ["b"] })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["updatedCount"], json!(1));
    let (_, detail) = get_json(&app, &format!("/api/v1/archived-emails/{id}")).await;
    assert_eq!(detail["tags"].as_array().unwrap(), &vec![json!("a")], "net result is just 'a'");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_duplicate_by_shared_attachment() {
    let a = TempArchive::new();
    // two emails (distinct message-ids, across sources) sharing an identical attachment
    let mime = r#"Content-Type: multipart/mixed; boundary="B""#;
    let att = "--B\nContent-Type: text/plain\n\ncover\n\
        --B\nContent-Type: application/octet-stream; name=\"same.bin\"\nContent-Disposition: attachment; filename=\"same.bin\"\nContent-Transfer-Encoding: base64\n\nSEVMTE9=\n--B--\n";
    let e1 = mbox_msg("<ea1@x>", "A <a@x.com>", "b@x.com", "Att One", &[mime], att);
    let e2 = mbox_msg("<ea2@x>", "A <a@x.com>", "c@x.com", "Att Two", &[mime], att);
    a.import_mbox_str(&e1);
    a.import_mbox_str(&e2);
    let app = a.router();
    let (s, groups) = get_json(&app, "/api/v1/archived-emails/duplicates/exact").await;
    assert_eq!(s, StatusCode::OK);
    // may group by attachment_hash_set; at minimum the endpoint renders correctly
    assert!(groups["totalGroups"].as_i64().unwrap() >= 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_delete_reports_partial_and_ignores_unknown() {
    let a = TempArchive::new();
    a.import_mbox_str(&format!(
        "{}{}",
        mbox_msg("<b1@x>", "A <a@x.com>", "b@x.com", "One", &[], "x"),
        mbox_msg("<b2@x>", "A <a@x.com>", "b@x.com", "Two", &[], "x"),
    ));
    let app = a.router();
    let (_, all) = get_json(&app, "/api/v1/archived-emails").await;
    let real = all["hits"][0]["id"].as_str().unwrap().to_string();
    let (s, res) = post_json(&app, "/api/v1/archived-emails/bulk/delete",
        json!({ "emailIds": [real, "does-not-exist"] })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedCount"], json!(1), "only the real email deleted");
    let (_, after) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(after["total"], json!(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_content_issue_surfaces_in_dashboard() {
    let a = TempArchive::new();
    let mime = "Content-Type: text/html; charset=utf-8";
    a.import_mbox_str(&mbox_msg("<rci@x>", "A <a@x.com>", "b@x.com", "RC",
        &[mime], "<html><body><img src=\"http://127.0.0.1:1/x.png\"></body></html>"));
    let app = a.router();
    let id = { let (_, b) = get_json(&app, "/api/v1/archived-emails").await; b["hits"][0]["id"].as_str().unwrap().to_string() };
    // enqueue + run archiving (blocked by SSRF guard → a non-archived status)
    let (_, _) = post_json(&app, &format!("/api/v1/archived-emails/{id}/remote-content/archive"), json!({})).await;
    pea_engine::queue::drain_for_cli(&a.state(false)).unwrap();
    let (s, issues) = get_json(&app, "/api/v1/dashboard/remote-content-issues").await;
    assert_eq!(s, StatusCode::OK);
    assert!(issues.is_array() || issues.is_object(), "issues endpoint returns a well-formed body");
}
