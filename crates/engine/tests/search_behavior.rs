//! Behavioral tests for the search API. Expected values come from FTS5
//! semantics + the SearchResult/ArchiveSearchField contract in packages/types,
//! NOT from whatever the code currently emits.
mod common;
use common::*;

fn subjects(body: &serde_json::Value) -> Vec<String> {
    body["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["subject"].as_str().unwrap_or("").to_string())
        .collect()
}
fn total(body: &serde_json::Value) -> i64 {
    body["total"].as_i64().unwrap()
}
fn nhits(body: &serde_json::Value) -> usize {
    body["hits"].as_array().unwrap().len()
}

/// multipart/mixed message with one text attachment.
fn msg_with_attachment(mid: &str, subject: &str, body: &str, filename: &str, att: &str) -> String {
    let ct = r#"Content-Type: multipart/mixed; boundary="XB""#;
    let mime = format!(
        "--XB\nContent-Type: text/plain; charset=utf-8\n\n{body}\n--XB\n\
         Content-Type: text/plain; name=\"{filename}\"\n\
         Content-Disposition: attachment; filename=\"{filename}\"\n\n{att}\n--XB--\n"
    );
    mbox_msg(mid, "Alice <alice@example.com>", "bob@example.com", subject, &[ct], &mime)
}

// ---- browse (no query) ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browse_with_no_query_returns_every_email() {
    let a = TempArchive::new();
    let mbox = format!(
        "{}{}{}",
        mbox_msg("<b1@x>", "Alice <alice@example.com>", "bob@x.com", "One", &[], "aaa"),
        mbox_msg("<b2@x>", "Alice <alice@example.com>", "bob@x.com", "Two", &[], "bbb"),
        mbox_msg("<b3@x>", "Alice <alice@example.com>", "bob@x.com", "Three", &[], "ccc"),
    );
    assert_eq!(a.import_mbox_str(&mbox), 3);
    let app = a.router();
    let (s, body) = get_json(&app, "/api/v1/archived-emails").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(total(&body), 3, "browse total = corpus size");
    assert_eq!(nhits(&body), 3);
}

// ---- field scoping: a term only in the body must not match fields=subject ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn field_scoping_restricts_to_named_fields() {
    let a = TempArchive::new();
    // subject "alphaword", body "zuluword" — disjoint tokens.
    let mbox = mbox_msg("<fs@x>", "Alice <alice@example.com>", "bob@x.com", "alphaword", &[], "zuluword here");
    a.import_mbox_str(&mbox);
    let app = a.router();

    let subj_only = |q: &str, f: &str| format!("/api/v1/archived-emails?q={q}&fields={f}");
    let (_, b) = get_json(&app, &subj_only("zuluword", "subject")).await;
    assert_eq!(total(&b), 0, "body-only term must NOT match fields=subject");
    let (_, b) = get_json(&app, &subj_only("zuluword", "body")).await;
    assert_eq!(total(&b), 1, "body-only term matches fields=body");
    let (_, b) = get_json(&app, &subj_only("alphaword", "subject")).await;
    assert_eq!(total(&b), 1, "subject term matches fields=subject");
    // default (no fields) searches everything
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=zuluword").await;
    assert_eq!(total(&b), 1, "default fields search body too");
}

// ---- prefix / search-as-you-type: the last term is a prefix ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_as_you_type_matches_prefixes() {
    let a = TempArchive::new();
    a.import_mbox_str(&mbox_msg("<p@x>", "Alice <a@x.com>", "b@x.com", "Sub", &[], "reprapper firmware"));
    let app = a.router();
    for q in ["repr", "reprap", "reprapper"] {
        let (_, b) = get_json(&app, &format!("/api/v1/archived-emails?q={q}")).await;
        assert_eq!(total(&b), 1, "prefix '{q}' matches 'reprapper'");
    }
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=xyzzy").await;
    assert_eq!(total(&b), 0, "non-matching prefix returns nothing");
}

// ---- hasAttachments filter ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn has_attachments_filter() {
    let a = TempArchive::new();
    let mbox = format!(
        "{}{}",
        msg_with_attachment("<a1@x>", "WithAtt", "sharedtoken body", "notes.txt", "attach text"),
        mbox_msg("<a2@x>", "Alice <a@x.com>", "b@x.com", "NoAtt", &[], "sharedtoken body"),
    );
    assert_eq!(a.import_mbox_str(&mbox), 2);
    let app = a.router();
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=sharedtoken&hasAttachments=true").await;
    assert_eq!(total(&b), 1, "only the attachment-bearing email");
    assert_eq!(subjects(&b), vec!["WithAtt"]);
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=sharedtoken&hasAttachments=false").await;
    assert_eq!(total(&b), 1, "only the attachment-free email");
    assert_eq!(subjects(&b), vec!["NoAtt"]);
}

// ---- from (sender) filter ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn from_filter_matches_sender_email() {
    let a = TempArchive::new();
    let mbox = format!(
        "{}{}",
        mbox_msg("<s1@x>", "Alice <alice@example.com>", "b@x.com", "FromAlice", &[], "sharedtoken"),
        mbox_msg("<s2@x>", "Carol <carol@example.com>", "b@x.com", "FromCarol", &[], "sharedtoken"),
    );
    a.import_mbox_str(&mbox);
    let app = a.router();
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=sharedtoken&from=alice@example.com").await;
    assert_eq!(total(&b), 1);
    assert_eq!(subjects(&b), vec!["FromAlice"]);
}

// ---- pagination: total is consistent across pages; slices don't overlap ----
//  (guards against COUNT(*) OVER () + bm25 dropping page 1's rows)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_total_consistent_and_slices_disjoint() {
    let a = TempArchive::new();
    let mut mbox = String::new();
    for tag in ["a", "b", "c", "d", "e"] {
        mbox.push_str(&mbox_msg(
            &format!("<pg{tag}@x>"),
            "Alice <a@x.com>",
            "b@x.com",
            &format!("wid {tag}"),
            &[],
            "widget body",
        ));
    }
    assert_eq!(a.import_mbox_str(&mbox), 5);
    let app = a.router();
    let page = |p: i32| format!("/api/v1/archived-emails?q=widget&sort=subject&direction=asc&limit=2&page={p}");

    let (_, p1) = get_json(&app, &page(1)).await;
    let (_, p2) = get_json(&app, &page(2)).await;
    let (_, p3) = get_json(&app, &page(3)).await;
    assert_eq!(total(&p1), 5, "page 1 total");
    assert_eq!(total(&p2), 5, "page 2 total — must equal page 1");
    assert_eq!(total(&p3), 5, "page 3 total");
    assert_eq!(subjects(&p1), vec!["wid a", "wid b"]);
    assert_eq!(subjects(&p2), vec!["wid c", "wid d"]);
    assert_eq!(subjects(&p3), vec!["wid e"]);
}

// ---- sort direction ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sort_by_subject_ascending_and_descending() {
    let a = TempArchive::new();
    let mbox = format!(
        "{}{}{}",
        mbox_msg("<z1@x>", "Alice <a@x.com>", "b@x.com", "Banana", &[], "fruittoken"),
        mbox_msg("<z2@x>", "Alice <a@x.com>", "b@x.com", "Apple", &[], "fruittoken"),
        mbox_msg("<z3@x>", "Alice <a@x.com>", "b@x.com", "Cherry", &[], "fruittoken"),
    );
    a.import_mbox_str(&mbox);
    let app = a.router();
    let (_, asc) = get_json(&app, "/api/v1/archived-emails?q=fruittoken&sort=subject&direction=asc").await;
    assert_eq!(subjects(&asc), vec!["Apple", "Banana", "Cherry"]);
    let (_, desc) = get_json(&app, "/api/v1/archived-emails?q=fruittoken&sort=subject&direction=desc").await;
    assert_eq!(subjects(&desc), vec!["Cherry", "Banana", "Apple"]);
}

// ---- matchingStrategy: AND, OR-fallback, strict ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn matching_strategy_and_or_fallback() {
    let a = TempArchive::new();
    let mbox = format!(
        "{}{}",
        mbox_msg("<m1@x>", "Alice <a@x.com>", "b@x.com", "M1", &[], "alphatok betatok"),
        mbox_msg("<m2@x>", "Alice <a@x.com>", "b@x.com", "M2", &[], "alphatok gammatok"),
    );
    a.import_mbox_str(&mbox);
    let app = a.router();

    // both terms present in one doc → AND matches exactly that one.
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=alphatok%20betatok").await;
    assert_eq!(total(&b), 1, "AND: only the doc with both terms");
    assert_eq!(subjects(&b), vec!["M1"]);

    // no doc has both → default falls back to OR (either term).
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=betatok%20gammatok").await;
    assert_eq!(total(&b), 2, "OR fallback when AND yields nothing");

    // strict 'all' disables the OR fallback → 0.
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=betatok%20gammatok&matchingStrategy=all").await;
    assert_eq!(total(&b), 0, "strict all: no OR fallback");
}

// ---- tags: filter + facets reflect a tag added via the API ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tags_filter_and_facets() {
    let a = TempArchive::new();
    let mbox = format!(
        "{}{}",
        mbox_msg("<t1@x>", "Alice <a@x.com>", "b@x.com", "Tagged", &[], "tagtoken"),
        mbox_msg("<t2@x>", "Alice <a@x.com>", "b@x.com", "Untagged", &[], "tagtoken"),
    );
    a.import_mbox_str(&mbox);
    let app = a.router();

    // find the id of "Tagged" and tag it "work".
    let (_, all) = get_json(&app, "/api/v1/archived-emails?q=tagtoken&sort=subject&direction=asc").await;
    let tagged_id = all["hits"][0]["id"].as_str().unwrap().to_string(); // "Tagged" < "Untagged"
    assert_eq!(all["hits"][0]["subject"], json!("Tagged"));
    let (s, _) = post_json(
        &app,
        "/api/v1/archived-emails/bulk/tags",
        json!({ "emailIds": [tagged_id], "addTags": ["work"] }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "tagging succeeds");

    // filter by tag
    let (_, b) = get_json(&app, "/api/v1/archived-emails?q=tagtoken&tags=work").await;
    assert_eq!(total(&b), 1, "tag filter narrows to the tagged email");
    assert_eq!(subjects(&b), vec!["Tagged"]);

    // facets expose the tag
    let (s, facets) = get_json(&app, "/api/v1/archived-emails/facets").await;
    assert_eq!(s, StatusCode::OK);
    let tags: Vec<String> = facets["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
    assert!(tags.contains(&"work".to_string()), "facets list the 'work' tag, got {tags:?}");
}

// ---- robustness: blank / punctuation queries must not error or dump everything wrongly ----
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blank_and_punctuation_queries_are_safe() {
    let a = TempArchive::new();
    a.import_mbox_str(&mbox_msg("<r1@x>", "Alice <a@x.com>", "b@x.com", "Sub", &[], "safetoken"));
    let app = a.router();

    // whitespace-only query is treated as "browse all"
    let (s, b) = get_json(&app, "/api/v1/archived-emails?q=%20%20").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(total(&b), 1, "blank query browses");

    // FTS metacharacters must not 500 or crash the query
    for q in ["%22", "%28%29", "AND", "*", "%3A"] {
        let (s, _) = get_json(&app, &format!("/api/v1/archived-emails?q={q}")).await;
        assert_eq!(s, StatusCode::OK, "query {q} did not error");
    }
}
