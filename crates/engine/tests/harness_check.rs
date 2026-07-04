//! Sanity check for the shared test harness itself.
mod common;
use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn harness_imports_crafted_mbox_and_serves() {
    let a = TempArchive::new();
    let n = a.import_mbox_str(&simple_mbox("Hello World", "body mentions reprap here"));
    assert_eq!(n, 1, "one crafted message archived");

    let app = a.router();
    let (s, body) = get_json(&app, "/api/v1/archived-emails?q=reprap").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["total"], json!(1), "crafted body is searchable");
    assert_eq!(body["hits"].as_array().unwrap().len(), 1);
}
