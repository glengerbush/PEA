//! Port of RemoteContentService's archive side: SSRF-guarded fetching of
//! remote images/stylesheets referenced by archived emails, stored encrypted
//! and recorded in remote_content_assets. Statuses and failure messages match
//! the Node implementation string-for-string.

use crate::preview::{self, normalize_content_type};
use crate::state::AppState;
use crate::{ingest, queue};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

const MAX_REMOTE_URLS_PER_EMAIL: usize = 50;
const MAX_REMOTE_CONTENT_BYTES: usize = 5 * 1024 * 1024;
const MAX_STYLESHEET_BYTES: usize = 1024 * 1024;
const FETCH_TIMEOUT_MS: u64 = 10_000;
const MAX_REDIRECTS: usize = 3;
const USER_AGENT: &str = "PEA-LocalRemoteContentArchiver/1.0";

fn hash_value(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn now_ms() -> i64 {
    crate::search::now_ms()
}

/// A fetch failure is either a policy block (→ asset status 'blocked') or a
/// transient error (→ 'failed').
enum FetchError {
    Blocked(String),
    Failed(String),
}

impl FetchError {
    fn message(&self) -> &str {
        match self {
            FetchError::Blocked(m) | FetchError::Failed(m) => m,
        }
    }
}

fn is_private_ipv4(octets: [u8; 4]) -> bool {
    let [a, b, _, _] = octets;
    a == 0
        || a == 10
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0)
        || (a == 192 && b == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51)
        || (a == 203 && b == 0)
        || a >= 224
}

fn is_blocked_ip(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => is_private_ipv4(v4.octets()),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_ipv4(v4.octets());
            }
            let seg = v6.segments();
            v6.is_unspecified()
                || v6.is_loopback()
                || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link local
                || (seg[0] & 0xff00) == 0xff00 // multicast
        }
    }
}

fn is_default_port(url: &url::Url) -> bool {
    match (url.scheme(), url.port()) {
        (_, None) => true,
        ("http", Some(80)) => true,
        ("https", Some(443)) => true,
        _ => false,
    }
}

/// assertSafeRemoteUrl — protocol/credentials/port/hostname checks + DNS
/// resolution with private-range blocking. Returns the pinned address.
fn assert_safe_remote_url(url: &url::Url) -> Result<IpAddr, FetchError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(FetchError::Blocked("Unsupported remote content protocol".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchError::Blocked("Credentialed remote content URLs are blocked".into()));
    }
    if !is_default_port(url) {
        return Err(FetchError::Blocked("Non-standard remote content ports are blocked".into()));
    }
    let hostname = url.host_str().unwrap_or("").to_lowercase();
    if hostname == "localhost" || hostname.ends_with(".localhost") {
        return Err(FetchError::Blocked("Localhost remote content is blocked".into()));
    }
    let lookup_host = hostname.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = lookup_host.parse::<IpAddr>() {
        if is_blocked_ip(&ip) {
            return Err(FetchError::Blocked(
                "Private or local network addresses are blocked".into(),
            ));
        }
        return Ok(ip);
    }
    // Node's dns.lookup THROWS on NXDOMAIN, which archiveRemoteAsset records as
    // a 'failed' asset (not 'blocked') with getaddrinfo's message — mirror it.
    let addresses: Vec<IpAddr> = std::net::ToSocketAddrs::to_socket_addrs(&(lookup_host, 80))
        .map_err(|_| FetchError::Failed(format!("getaddrinfo ENOTFOUND {lookup_host}")))?
        .map(|sa| sa.ip())
        .collect();
    if addresses.is_empty() {
        return Err(FetchError::Blocked("Remote content host did not resolve".into()));
    }
    if addresses.iter().any(is_blocked_ip) {
        return Err(FetchError::Blocked(
            "Private or local network addresses are blocked".into(),
        ));
    }
    // Prefer IPv4 (v6 routes are often absent).
    addresses
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
        .ok_or_else(|| FetchError::Blocked("Remote content host did not resolve".into()))
}

fn looks_like_text(body: &[u8]) -> bool {
    let sample = &body[..body.len().min(4096)];
    let mut control = 0usize;
    for byte in sample {
        if *byte == 0 {
            return false;
        }
        if *byte < 9 || (*byte > 13 && *byte < 32) {
            control += 1;
        }
    }
    (control as f64) / (sample.len().max(1) as f64) < 0.1
}

fn detect_image_content_type(body: &[u8]) -> Option<&'static str> {
    if body.len() >= 8 && body[..8] == [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a] {
        return Some("image/png");
    }
    if body.len() >= 3 && body[..3] == [0xff, 0xd8, 0xff] {
        return Some("image/jpeg");
    }
    if body.len() >= 6 && (&body[..6] == b"GIF87a" || &body[..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if body.len() >= 12 && &body[..4] == b"RIFF" && &body[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if body.len() >= 12 && &body[4..12] == b"ftypavif" {
        return Some("image/avif");
    }
    None
}

fn validate_archivable(body: &[u8], content_type_header: Option<&str>) -> Result<String, FetchError> {
    if body.is_empty() {
        return Err(FetchError::Blocked("Remote content is empty".into()));
    }
    if let Some(sniffed) = detect_image_content_type(body) {
        return Ok(sniffed.to_string());
    }
    if normalize_content_type(content_type_header).as_deref() == Some("text/css") {
        if body.len() > MAX_STYLESHEET_BYTES {
            return Err(FetchError::Blocked("Remote stylesheet is too large".into()));
        }
        if !looks_like_text(body) {
            return Err(FetchError::Blocked("Remote stylesheet is not text".into()));
        }
        return Ok("text/css".into());
    }
    Err(FetchError::Blocked("Remote content type is not archivable".into()))
}

struct Fetched {
    body: Vec<u8>,
    content_type: String,
    final_url: String,
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "image/avif" => ".avif",
        _ => ".bin",
    }
}

/// fetchRemoteContent — blocking reqwest with manual redirects so every hop
/// re-runs the SSRF checks, pinned to the resolved address.
fn fetch_remote_content(raw_url: &str, redirect_count: usize) -> Result<Fetched, FetchError> {
    if redirect_count > MAX_REDIRECTS {
        return Err(FetchError::Blocked("Too many redirects".into()));
    }
    let url = url::Url::parse(raw_url)
        .map_err(|e| FetchError::Failed(format!("Invalid URL: {e}")))?;
    let address = assert_safe_remote_url(&url)?;

    let port = url.port_or_known_default().unwrap_or(80);
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(FETCH_TIMEOUT_MS))
        .resolve(
            url.host_str().unwrap_or(""),
            std::net::SocketAddr::new(address, port),
        )
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| FetchError::Failed(e.to_string()))?;
    let response = client
        .get(url.as_str())
        .header(
            reqwest::header::ACCEPT,
            "image/avif,image/webp,image/png,image/jpeg,image/gif,*/*;q=0.1",
        )
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                FetchError::Blocked("Remote content fetch timed out".into())
            } else {
                FetchError::Failed(e.to_string())
            }
        })?;

    let status = response.status().as_u16();
    if [301, 302, 303, 307, 308].contains(&status) {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| FetchError::Blocked("Redirect without location header".into()))?;
        let redirected = url
            .join(&location)
            .map_err(|e| FetchError::Failed(e.to_string()))?;
        return fetch_remote_content(redirected.as_str(), redirect_count + 1);
    }
    if !(200..300).contains(&status) {
        return Err(FetchError::Failed(format!("Remote server returned {status}")));
    }
    if let Some(len) = response.content_length() {
        if len as usize > MAX_REMOTE_CONTENT_BYTES {
            return Err(FetchError::Blocked("Remote content is too large".into()));
        }
    }
    let content_type_header = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body = response
        .bytes()
        .map_err(|e| FetchError::Failed(e.to_string()))?
        .to_vec();
    if body.len() > MAX_REMOTE_CONTENT_BYTES {
        return Err(FetchError::Blocked("Remote content is too large".into()));
    }
    let content_type = validate_archivable(&body, content_type_header.as_deref())?;
    Ok(Fetched { body, content_type, final_url: url.to_string() })
}

/// enqueueRemoteContentArchive — flips emails to 'pending' and queues a batch.
pub fn enqueue_archive(state: &AppState, email_ids: &[String]) -> Option<String> {
    let mut unique: Vec<String> = Vec::new();
    for id in email_ids {
        if !id.is_empty() && !unique.contains(id) {
            unique.push(id.clone());
        }
    }
    if !unique.is_empty() {
        let conn = state.pool.get().ok()?;
        let placeholders = vec!["?"; unique.len()].join(", ");
        conn.execute(
            &format!(
                "UPDATE archived_emails SET remote_content_status = 'pending' WHERE id IN ({placeholders})"
            ),
            rusqlite::params_from_iter(unique.iter()),
        )
        .ok();
    }
    queue::send_job(
        state,
        "remote-content",
        "archive-remote-content-batch",
        &json!({ "emailIds": unique }),
        queue::SendOptions::default(),
    )
}

fn archive_remote_asset(state: &AppState, conn: &Connection, email_id: &str, original_url: &str) {
    let url_hash = hash_value(original_url.as_bytes());
    let existing: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT id, status, storage_path FROM remote_content_assets \
             WHERE email_id = ? AND url_hash = ?",
            rusqlite::params![email_id, url_hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    if let Some((_, status, storage_path)) = &existing {
        if status == "archived" && storage_path.is_some() {
            return;
        }
    }
    let asset_id = match existing {
        Some((id, _, _)) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT OR IGNORE INTO remote_content_assets (id, email_id, original_url, url_hash, status) \
                 VALUES (?, ?, ?, ?, 'pending')",
                rusqlite::params![id, email_id, original_url, url_hash],
            )
            .ok();
            // Re-read in case a concurrent writer won the insert race.
            match conn.query_row(
                "SELECT id FROM remote_content_assets WHERE email_id = ? AND url_hash = ?",
                rusqlite::params![email_id, url_hash],
                |r| r.get::<_, String>(0),
            ) {
                Ok(id) => id,
                Err(_) => return,
            }
        }
    };

    match fetch_remote_content(original_url, 0) {
        Ok(fetched) => {
            let content_hash = hash_value(&fetched.body);
            let storage_path = format!(
                "open-archiver/remote-content/{email_id}/{asset_id}{}",
                extension_for(&fetched.content_type)
            );
            if state.storage_put(&storage_path, &fetched.body).is_err() {
                return;
            }
            conn.execute(
                "UPDATE remote_content_assets SET final_url = ?, status = 'archived', \
                 content_type = ?, size_bytes = ?, content_hash_sha256 = ?, storage_path = ?, \
                 failure_reason = NULL, updated_at = ? WHERE id = ?",
                rusqlite::params![
                    fetched.final_url,
                    fetched.content_type,
                    fetched.body.len() as i64,
                    content_hash,
                    storage_path,
                    now_ms(),
                    asset_id
                ],
            )
            .ok();
        }
        Err(error) => {
            let status = match &error {
                FetchError::Blocked(_) => "blocked",
                FetchError::Failed(_) => "failed",
            };
            conn.execute(
                "UPDATE remote_content_assets SET status = ?, failure_reason = ?, updated_at = ? \
                 WHERE id = ?",
                rusqlite::params![status, error.message(), now_ms(), asset_id],
            )
            .ok();
        }
    }
}

/// Second pass — archive url(...) images referenced by archived stylesheets.
fn archive_stylesheet_subresources(state: &AppState, conn: &Connection, email_id: &str) {
    static URL_PAT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"(?i)url\(\s*(?:"([^"]+)"|'([^']+)'|([^)]+))\s*\)"#).unwrap()
    });
    let mut stmt = match conn.prepare(
        "SELECT url_hash, status, storage_path, content_type, final_url, original_url \
         FROM remote_content_assets WHERE email_id = ?",
    ) {
        Ok(s) => s,
        Err(_) => return,
    };
    struct AssetRow {
        url_hash: String,
        status: String,
        storage_path: Option<String>,
        content_type: Option<String>,
        final_url: Option<String>,
        original_url: String,
    }
    let assets: Vec<AssetRow> = stmt
        .query_map([email_id], |r| {
            Ok(AssetRow {
                url_hash: r.get(0)?,
                status: r.get(1)?,
                storage_path: r.get(2)?,
                content_type: r.get(3)?,
                final_url: r.get(4)?,
                original_url: r.get(5)?,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();

    let mut known: std::collections::HashSet<String> =
        assets.iter().map(|a| a.url_hash.clone()).collect();
    let mut sub_urls: Vec<String> = Vec::new();
    for sheet in assets.iter().filter(|a| {
        a.status == "archived"
            && a.storage_path.is_some()
            && normalize_content_type(a.content_type.as_deref()).as_deref() == Some("text/css")
    }) {
        let Ok(bytes) = state.storage_get(sheet.storage_path.as_deref().unwrap()) else {
            continue;
        };
        let css = String::from_utf8_lossy(&bytes);
        let base = sheet.final_url.clone().unwrap_or_else(|| sheet.original_url.clone());
        for m in URL_PAT.captures_iter(&css) {
            let raw = m
                .get(1)
                .or_else(|| m.get(2))
                .or_else(|| m.get(3))
                .map(|g| g.as_str().trim())
                .unwrap_or("");
            if raw.is_empty() || raw.starts_with("data:") || raw.starts_with('#') {
                continue;
            }
            let resolved = url::Url::parse(&base)
                .ok()
                .and_then(|b| b.join(raw).ok())
                .filter(|u| u.scheme() == "http" || u.scheme() == "https")
                .map(|u| u.to_string());
            let Some(resolved) = resolved else { continue };
            if preview::is_likely_tracking_url_pub(&resolved) {
                continue;
            }
            let h = hash_value(resolved.as_bytes());
            if known.contains(&h) {
                continue;
            }
            known.insert(h);
            if !sub_urls.contains(&resolved) {
                sub_urls.push(resolved);
            }
        }
    }
    for url in sub_urls.into_iter().take(MAX_REMOTE_URLS_PER_EMAIL) {
        archive_remote_asset(state, conn, email_id, &url);
    }
}

fn summarize_status(archived: i64, failed: i64, blocked: i64) -> &'static str {
    let resolved = archived + failed + blocked;
    if resolved == 0 {
        return "skipped";
    }
    if failed == 0 && blocked == 0 {
        return "archived";
    }
    if archived > 0 {
        return "partial";
    }
    "failed"
}

fn archive_email_remote_content(state: &AppState, conn: &Connection, email_id: &str) -> Result<(), String> {
    let storage_path: Option<String> = conn
        .query_row(
            "SELECT storage_path FROM archived_emails WHERE id = ?",
            [email_id],
            |r| r.get(0),
        )
        .ok();
    let Some(storage_path) = storage_path else { return Ok(()) };

    conn.execute(
        "UPDATE archived_emails SET remote_content_status = 'pending' WHERE id = ?",
        [email_id],
    )
    .ok();

    let raw = state.storage_get(&storage_path)?;
    let html = match mail_parser::MessageParser::default().parse(&raw) {
        Some(msg) => ingest::mailparser_text_and_html(&msg).1,
        None => String::new(),
    };
    let remote_urls: Vec<String> = preview::extract_remote_urls_ordered(&html)
        .into_iter()
        .take(MAX_REMOTE_URLS_PER_EMAIL)
        .collect();

    if remote_urls.is_empty() {
        conn.execute(
            "UPDATE archived_emails SET remote_content_status = 'skipped', \
             remote_content_asset_count = 0, remote_content_archived_at = ? WHERE id = ?",
            rusqlite::params![now_ms(), email_id],
        )
        .ok();
        return Ok(());
    }

    for url in &remote_urls {
        archive_remote_asset(state, conn, email_id, url);
    }
    archive_stylesheet_subresources(state, conn, email_id);

    let (archived, failed, blocked): (i64, i64, i64) = conn
        .query_row(
            "SELECT count(*) FILTER (WHERE status = 'archived'), \
             count(*) FILTER (WHERE status = 'failed'), \
             count(*) FILTER (WHERE status = 'blocked') \
             FROM remote_content_assets WHERE email_id = ?",
            [email_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or((0, 0, 0));
    conn.execute(
        "UPDATE archived_emails SET remote_content_status = ?, remote_content_asset_count = ?, \
         remote_content_archived_at = ? WHERE id = ?",
        rusqlite::params![summarize_status(archived, failed, blocked), archived, now_ms(), email_id],
    )
    .ok();
    Ok(())
}

/// archive-remote-content-batch processor.
pub fn archive_batch(state: &AppState, payload: &Value) -> Result<(), String> {
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let ids: Vec<String> = payload
        .get("emailIds")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut unique: Vec<String> = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    for email_id in unique {
        if let Err(e) = archive_email_remote_content(state, &conn, &email_id) {
            conn.execute(
                "UPDATE archived_emails SET remote_content_status = 'failed', \
                 remote_content_archived_at = ? WHERE id = ?",
                rusqlite::params![now_ms(), email_id],
            )
            .ok();
            eprintln!("[remote-content] archive failed for {email_id}: {e}");
        }
    }
    Ok(())
}
