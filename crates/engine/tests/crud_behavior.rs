//! Behavioral tests for the mutation layer (writes.rs handlers). Expected
//! values come from the *Result type contracts in packages/types and the
//! documented semantics (tags are a set; delete cascades; settings merge),
//! not from the current implementation.
mod common;
use common::*;

async fn one_email_id(app: &axum::Router) -> String {
    let (_, b) = get_json(app, "/api/v1/archived-emails").await;
    b["hits"][0]["id"].as_str().unwrap().to_string()
}
fn tag_set(v: &serde_json::Value) -> Vec<String> {
    let mut t: Vec<String> = v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    t.sort();
    t
}

// ---- tags are a SET: add is idempotent, remove works, no duplicates ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tags_use_set_semantics() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "body"));
    let app = a.router();
    let id = one_email_id(&app).await;

    let (s, _) = post_json(&app, "/api/v1/archived-emails/bulk/tags",
        json!({ "emailIds": [id], "addTags": ["work", "urgent"] })).await;
    assert_eq!(s, StatusCode::OK);

    // adding "work" again must NOT create a duplicate
    let (_, _) = post_json(&app, "/api/v1/archived-emails/bulk/tags",
        json!({ "emailIds": [id], "addTags": ["work"] })).await;
    let (_, detail) = get_json(&app, &format!("/api/v1/archived-emails/{id}")).await;
    assert_eq!(tag_set(&detail["tags"]), vec!["urgent", "work"], "no duplicate 'work'");

    // remove one
    let (_, _) = post_json(&app, "/api/v1/archived-emails/bulk/tags",
        json!({ "emailIds": [id], "removeTags": ["urgent"] })).await;
    let (_, detail) = get_json(&app, &format!("/api/v1/archived-emails/{id}")).await;
    assert_eq!(tag_set(&detail["tags"]), vec!["work"], "urgent removed, work kept");
}

// ---- bulk delete removes the requested emails and reports the count ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_delete_removes_and_reports() {
    let a = TempArchive::new();
    let mbox = format!(
        "{}{}{}",
        mbox_msg("<d1@x>", "Alice <a@x.com>", "b@x.com", "D1", &[], "x"),
        mbox_msg("<d2@x>", "Alice <a@x.com>", "b@x.com", "D2", &[], "x"),
        mbox_msg("<d3@x>", "Alice <a@x.com>", "b@x.com", "D3", &[], "x"),
    );
    a.import_mbox_str(&mbox);
    let app = a.router();
    let (_, all) = get_json(&app, "/api/v1/archived-emails").await;
    let ids: Vec<String> = all["hits"].as_array().unwrap().iter().map(|h| h["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 3);

    let (s, res) = post_json(&app, "/api/v1/archived-emails/bulk/delete",
        json!({ "emailIds": [ids[0], ids[1]] })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(res["deletedCount"], json!(2), "reports two deleted");

    let (_, after) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(after["total"], json!(1), "one email remains");
    assert_eq!(after["hits"][0]["id"], json!(ids[2]), "the untouched one remains");
}

// ---- settings: partial PUT merges over stored config and persists ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_merge_and_persist() {
    let a = TempArchive::new();
    let app = a.router();
    let (_, def) = get_json(&app, "/api/v1/settings/system").await;
    assert_eq!(def["theme"], json!("system"));
    assert_eq!(def["clockFormat"], json!("12h"));

    let (s, put) = put_json(&app, "/api/v1/settings/system", json!({ "theme": "dark", "clockFormat": "24h" })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(put["theme"], json!("dark"));
    assert_eq!(put["language"], json!("en"), "unspecified keys keep defaults");

    // a second partial update must preserve the earlier change
    let (_, put2) = put_json(&app, "/api/v1/settings/system", json!({ "theme": "light" })).await;
    assert_eq!(put2["theme"], json!("light"));
    assert_eq!(put2["clockFormat"], json!("24h"), "prior clockFormat survived the merge");

    let (_, after) = get_json(&app, "/api/v1/settings/system").await;
    assert_eq!(after["theme"], json!("light"), "persisted across requests");
    assert_eq!(after["clockFormat"], json!("24h"));
}

// ---- source lifecycle: rename → pause → delete cascades to emails ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_rename_pause_delete_cascade() {
    let a = TempArchive::new();
    a.import_mbox_str(&simple_mbox("Sub", "cascadetoken"));
    let app = a.router();

    let (_, sources) = get_json(&app, "/api/v1/ingestion-sources").await;
    assert_eq!(sources.as_array().unwrap().len(), 1);
    let id = sources[0]["id"].as_str().unwrap().to_string();

    let (s, renamed) = put_json(&app, &format!("/api/v1/ingestion-sources/{id}"), json!({ "name": "Renamed" })).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(renamed["name"], json!("Renamed"));

    let (s, paused) = post_json(&app, &format!("/api/v1/ingestion-sources/{id}/pause"), json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(paused["status"], json!("paused"));

    // unmerge on a non-merged source is a 400
    let (s, _) = post_json(&app, &format!("/api/v1/ingestion-sources/{id}/unmerge"), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "cannot unmerge a root source");

    // delete cascades: source gone AND its emails gone
    let (s, _) = send(&app, "DELETE", &format!("/api/v1/ingestion-sources/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, sources) = get_json(&app, "/api/v1/ingestion-sources").await;
    assert_eq!(sources.as_array().unwrap().len(), 0, "source removed");
    let (_, emails) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(emails["total"], json!(0), "emails cascaded away");
}

// ---- bad requests are rejected with 400, not 500 ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutations_reject_bad_input() {
    let a = TempArchive::new();
    let app = a.router();
    // tags without emailIds
    let (s, _) = post_json(&app, "/api/v1/archived-emails/bulk/tags", json!({ "addTags": ["x"] })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // delete without emailIds
    let (s, _) = post_json(&app, "/api/v1/archived-emails/bulk/delete", json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
