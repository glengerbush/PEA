//! Archive side of remote content: SSRF-guarded fetching of remote
//! images/stylesheets referenced by archived emails, stored on disk and
//! recorded in remote_content_assets. The human-readable block/fail reasons
//! name the offending type / size / address.

use crate::preview::{self, normalize_content_type};
use crate::state::AppState;
use crate::{ingest, queue};
use std::sync::LazyLock;
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

const MAX_REMOTE_URLS_PER_EMAIL: usize = 50;
const MAX_REMOTE_CONTENT_BYTES: usize = 5 * 1024 * 1024;
const MAX_STYLESHEET_BYTES: usize = 1024 * 1024;
const FETCH_TIMEOUT_MS: u64 = 10_000;
const MAX_REDIRECTS: usize = 3;
// Present as a mainstream desktop browser rather than a bot. Remote email content
// (logos, product images, stylesheets) is frequently served only to browser-like
// clients; PEA fetches each asset exactly once, on the user's behalf, to archive an
// email they already received — so identifying as a normal browser is what actually
// retrieves the content they're entitled to. (This can't defeat JS/Captcha
// challenges, which PEA doesn't execute.)
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

fn hash_value(data: &[u8]) -> String {
    crate::hex_encode(Sha256::digest(data))
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
    let [a, b, c, _] = octets;
    a == 0
        || a == 10
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0) // 192.0.0.0/24 IETF protocol assignments
        || (a == 192 && b == 0 && c == 2) // 192.0.2.0/24 TEST-NET-1
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224
}

fn is_blocked_ip(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => is_private_ipv4(v4.octets()),
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            // Decode any embedded IPv4 and apply the v4 policy: IPv4-mapped
            // (::ffff:a.b.c.d), IPv4-compatible (::a.b.c.d), 6to4 (2002:AABB:CCDD::/16),
            // and the NAT64 well-known prefix (64:ff9b::/96) — otherwise a literal
            // like `::127.0.0.1` would slip past the loopback check below.
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| v6.to_ipv4()) {
                return is_private_ipv4(v4.octets());
            }
            if seg[0] == 0x2002 {
                return is_private_ipv4([
                    (seg[1] >> 8) as u8, seg[1] as u8, (seg[2] >> 8) as u8, seg[2] as u8,
                ]);
            }
            if seg[0] == 0x0064 && seg[1] == 0xff9b {
                return is_private_ipv4([
                    (seg[6] >> 8) as u8, seg[6] as u8, (seg[7] >> 8) as u8, seg[7] as u8,
                ]);
            }
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

/// Protocol/credentials/port/hostname checks + DNS resolution with
/// private-range blocking. Returns the pinned address.
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
            return Err(FetchError::Blocked(format!(
                "Address {ip} is a private or local network address"
            )));
        }
        return Ok(ip);
    }
    // A name that doesn't resolve is a transient/technical failure — recorded as
    // 'failed', not 'blocked'.
    let addresses: Vec<IpAddr> = std::net::ToSocketAddrs::to_socket_addrs(&(lookup_host, 80))
        .map_err(|_| {
            FetchError::Failed(format!("Could not resolve host '{lookup_host}' (DNS lookup failed)"))
        })?
        .map(|sa| sa.ip())
        .collect();
    if addresses.is_empty() {
        return Err(FetchError::Blocked("Remote content host did not resolve".into()));
    }
    if let Some(blocked) = addresses.iter().find(|a| is_blocked_ip(a)) {
        return Err(FetchError::Blocked(format!(
            "{hostname} resolves to a private or local address ({blocked})"
        )));
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

/// Human-readable byte size for failure reasons (e.g. "8.4 MB").
fn human_size(bytes: usize) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 0.1 {
        format!("{mb:.1} MB")
    } else {
        format!("{bytes} bytes")
    }
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
            return Err(FetchError::Blocked(format!(
                "Remote stylesheet is {} (over the {} limit)",
                human_size(body.len()),
                human_size(MAX_STYLESHEET_BYTES)
            )));
        }
        if !looks_like_text(body) {
            return Err(FetchError::Blocked("Remote stylesheet is not valid text".into()));
        }
        return Ok("text/css".into());
    }
    // Not a recognized image and not archivable CSS — name the actual type so the
    // reason is specific (SVG images, web fonts and HTML are the usual culprits).
    let reported = normalize_content_type(content_type_header)
        .or_else(|| content_type_header.map(|s| s.trim().to_lowercase()))
        .filter(|s| !s.is_empty());
    let message = match reported {
        Some(t) if t.starts_with("image/") => format!(
            "Remote image type '{t}' is not supported (archives PNG, JPEG, GIF, WebP, AVIF)"
        ),
        Some(t) => format!("Remote content type '{t}' is not archivable (only images and CSS)"),
        None => "Remote content has no content type and is not a recognized image".to_string(),
    };
    Err(FetchError::Blocked(message))
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

/// The innermost cause of a reqwest error. Its top-level Display is a verbose
/// chain ("error sending request for url (...): ...: Connection refused"), while
/// the deepest source ("Connection refused (os error 111)") is the useful part.
fn innermost_cause(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    msg
}

/// A readable reason for a non-success HTTP status, with the canonical reason
/// phrase where the code has one (e.g. "Remote server returned HTTP 404 Not Found").
fn describe_http_status(status: reqwest::StatusCode) -> String {
    match status.canonical_reason() {
        Some(reason) => format!("Remote server returned HTTP {} {reason}", status.as_u16()),
        None => format!("Remote server returned HTTP {}", status.as_u16()),
    }
}

/// Blocking reqwest fetch with manual redirects so every hop re-runs the SSRF
/// checks, pinned to the resolved address.
fn fetch_remote_content(raw_url: &str, redirect_count: usize) -> Result<Fetched, FetchError> {
    if redirect_count > MAX_REDIRECTS {
        return Err(FetchError::Blocked("Too many redirects".into()));
    }
    let url = url::Url::parse(raw_url)
        .map_err(|e| FetchError::Failed(format!("Invalid remote content URL: {e}")))?;
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
        .map_err(|e| FetchError::Failed(format!("Could not start remote fetch: {e}")))?;
    let mut response = client
        .get(url.as_str())
        // Browser-like request headers, matching the User-Agent, so senders that
        // gate remote content on a real-browser fingerprint still serve it.
        .header(
            reqwest::header::ACCEPT,
            "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .header("Sec-Fetch-Dest", "image")
        .header("Sec-Fetch-Mode", "no-cors")
        .header("Sec-Fetch-Site", "cross-site")
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                FetchError::Blocked("Remote content fetch timed out".into())
            } else if e.is_connect() {
                FetchError::Failed(format!(
                    "Could not connect to the remote server: {}",
                    innermost_cause(&e)
                ))
            } else {
                FetchError::Failed(format!("Remote content request failed: {}", innermost_cause(&e)))
            }
        })?;

    let status_code = response.status();
    let status = status_code.as_u16();
    if [301, 302, 303, 307, 308].contains(&status) {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| FetchError::Blocked("Redirect without location header".into()))?;
        let redirected = url
            .join(&location)
            .map_err(|_| FetchError::Failed(format!("Invalid redirect target '{location}'")))?;
        return fetch_remote_content(redirected.as_str(), redirect_count + 1);
    }
    if !(200..300).contains(&status) {
        return Err(FetchError::Failed(describe_http_status(status_code)));
    }
    if let Some(len) = response.content_length() {
        if len as usize > MAX_REMOTE_CONTENT_BYTES {
            return Err(FetchError::Blocked(format!(
                "Remote content is {} (over the {} limit)",
                human_size(len as usize),
                human_size(MAX_REMOTE_CONTENT_BYTES)
            )));
        }
    }
    let content_type_header = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    // Stream with a hard cap: a chunked response with no Content-Length would
    // otherwise read an unbounded body into memory before any size check.
    let mut body: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = std::io::Read::read(&mut response, &mut buf)
            .map_err(|e| FetchError::Failed(format!("Error reading remote content: {e}")))?;
        if n == 0 {
            break;
        }
        if body.len() + n > MAX_REMOTE_CONTENT_BYTES {
            return Err(FetchError::Blocked(format!(
                "Remote content exceeds the {} limit",
                human_size(MAX_REMOTE_CONTENT_BYTES)
            )));
        }
        body.extend_from_slice(&buf[..n]);
    }
    let content_type = validate_archivable(&body, content_type_header.as_deref())?;
    Ok(Fetched { body, content_type, final_url: url.to_string() })
}

/// Flips emails to 'pending' and queues an archive batch.
pub fn enqueue_archive(state: &AppState, email_ids: &[String]) -> Option<String> {
    let mut unique: Vec<String> = Vec::new();
    for id in email_ids {
        if !id.is_empty() && !unique.contains(id) {
            unique.push(id.clone());
        }
    }
    // Enqueue FIRST, then flip to 'pending' only if the job was actually queued —
    // marking pending before a (possibly failing) enqueue would strand emails
    // showing "pending" with no worker to ever process them.
    let job_id = queue::send_job(
        state,
        "remote-content",
        "archive-remote-content-batch",
        &json!({ "emailIds": unique }),
        queue::SendOptions::default(),
    );
    if job_id.is_some() && !unique.is_empty() {
        if let Ok(conn) = state.pool.get() {
            let placeholders = vec!["?"; unique.len()].join(", ");
            conn.execute(
                &format!(
                    "UPDATE archived_emails SET remote_content_status = 'pending' WHERE id IN ({placeholders})"
                ),
                rusqlite::params_from_iter(unique.iter()),
            )
            .ok();
        }
    }
    job_id
}

fn archive_remote_asset(state: &AppState, email_id: &str, original_url: &str) {
    let url_hash = hash_value(original_url.as_bytes());
    // Reserve the asset row, then RELEASE the pooled connection before the slow,
    // blocking network fetch below — holding one across fetches starves the
    // small connection pool during a large archive run.
    let asset_id = {
        let Ok(conn) = state.pool.get() else { return };
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
        match existing {
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
        }
    };

    let fetched = fetch_remote_content(original_url, 0);

    let Ok(conn) = state.pool.get() else { return };
    match fetched {
        Ok(fetched) => {
            let storage_path = format!(
                "pea/remote-content/{email_id}/{asset_id}{}",
                extension_for(&fetched.content_type)
            );
            if let Err(e) = state.storage_put(&storage_path, &fetched.body) {
                // Reach a terminal state so the URL is counted, not stuck pending.
                conn.execute(
                    "UPDATE remote_content_assets SET status = 'failed', failure_reason = ? WHERE id = ?",
                    rusqlite::params![format!("storage error: {e}"), asset_id],
                )
                .ok();
                return;
            }
            conn.execute(
                "UPDATE remote_content_assets SET final_url = ?, status = 'archived', \
                 content_type = ?, size_bytes = ?, storage_path = ?, \
                 failure_reason = NULL WHERE id = ?",
                rusqlite::params![
                    fetched.final_url,
                    fetched.content_type,
                    fetched.body.len() as i64,
                    storage_path,
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
                "UPDATE remote_content_assets SET status = ?, failure_reason = ? \
                 WHERE id = ?",
                rusqlite::params![status, error.message(), asset_id],
            )
            .ok();
        }
    }
}

/// Second pass — archive url(...) images referenced by archived stylesheets.
fn archive_stylesheet_subresources(state: &AppState, email_id: &str) {
    static URL_PAT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)url\(\s*(?:"([^"]+)"|'([^']+)'|([^)]+))\s*\)"#).unwrap()
    });
    struct AssetRow {
        url_hash: String,
        status: String,
        storage_path: Option<String>,
        content_type: Option<String>,
        final_url: Option<String>,
        original_url: String,
    }
    // Read the asset rows in a short scope and drop the connection: the CSS
    // parsing and sub-resource fetches below must not pin a pooled connection.
    let assets: Vec<AssetRow> = {
        let Ok(conn) = state.pool.get() else { return };
        let mut stmt = match conn.prepare(
            "SELECT url_hash, status, storage_path, content_type, final_url, original_url \
             FROM remote_content_assets WHERE email_id = ?",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        stmt.query_map([email_id], |r| {
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
        .unwrap_or_default()
    };

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
        archive_remote_asset(state, email_id, &url);
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

fn archive_email_remote_content(state: &AppState, email_id: &str) -> Result<(), String> {
    let storage_path: Option<String> = {
        let Ok(conn) = state.pool.get() else { return Ok(()) };
        conn.query_row(
            "SELECT storage_path FROM archived_emails WHERE id = ?",
            [email_id],
            |r| r.get(0),
        )
        .ok()
    };
    let Some(storage_path) = storage_path else { return Ok(()) };

    if let Ok(conn) = state.pool.get() {
        conn.execute(
            "UPDATE archived_emails SET remote_content_status = 'pending' WHERE id = ?",
            [email_id],
        )
        .ok();
    }

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
        if let Ok(conn) = state.pool.get() {
            conn.execute(
                "UPDATE archived_emails SET remote_content_status = 'skipped', \
                 remote_content_asset_count = 0, remote_content_archived_at = ? WHERE id = ?",
                rusqlite::params![now_ms(), email_id],
            )
            .ok();
        }
        return Ok(());
    }

    // Each of these acquires/releases a connection per DB touch; the network
    // fetches they perform hold no pooled connection.
    for url in &remote_urls {
        archive_remote_asset(state, email_id, url);
    }
    archive_stylesheet_subresources(state, email_id);

    // Remove any failed/blocked assets whose URLs match the tracking filter,
    // then recompute the status from what remains.
    prune_tracking_assets(state, email_id);
    recompute_email_status(state, email_id);
    Ok(())
}

/// Recompute an email's aggregate remote-content status + asset count from its
/// current assets. Shared by the full archive pass and single-asset retry.
fn recompute_email_status(state: &AppState, email_id: &str) {
    let Ok(conn) = state.pool.get() else { return };
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
}

/// Retry a SINGLE remote asset by its original URL: re-fetch just this one, then
/// recompute the email's status. archive_remote_asset skips already-archived
/// assets, so a retry never re-fetches or overwrites content that already
/// succeeded — only the targeted failed/blocked asset is re-attempted.
pub fn retry_asset(state: &AppState, email_id: &str, original_url: &str) {
    archive_remote_asset(state, email_id, original_url);
    recompute_email_status(state, email_id);
}

/// Delete an email's failed/blocked remote assets whose URL matches the tracking
/// filter. Only failed/blocked rows are removed — they have no stored file — so
/// this is storage-safe and never touches archived content. Returns the count.
fn prune_tracking_assets(state: &AppState, email_id: &str) -> usize {
    let Ok(conn) = state.pool.get() else { return 0 };
    let rows: Vec<(String, String)> = {
        let mut stmt = match conn.prepare(
            "SELECT id, original_url FROM remote_content_assets \
             WHERE email_id = ? AND status IN ('failed', 'blocked')",
        ) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        stmt.query_map([email_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    let mut removed = 0;
    for (asset_id, url) in rows {
        if preview::is_likely_tracking_url_pub(&url)
            && conn
                .execute("DELETE FROM remote_content_assets WHERE id = ?", [&asset_id])
                .unwrap_or(0)
                > 0
        {
            removed += 1;
        }
    }
    removed
}

/// One-shot cleanup across ALL emails: remove failed/blocked remote assets whose
/// URLs match the tracking filter and recompute the affected emails' statuses.
/// Self-limiting — once cleaned, later runs find nothing — and cheap (bounded by
/// the failed/blocked asset count).
pub fn sweep_tracking_assets(state: &AppState) {
    let affected: Vec<String> = {
        let Ok(conn) = state.pool.get() else { return };
        let mut stmt = match conn.prepare(
            "SELECT DISTINCT email_id FROM remote_content_assets WHERE status IN ('failed', 'blocked')",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    let mut total = 0usize;
    for email_id in &affected {
        let removed = prune_tracking_assets(state, email_id);
        if removed > 0 {
            recompute_email_status(state, email_id);
            total += removed;
        }
    }
    if total > 0 {
        eprintln!("[remote-content] swept {total} stale tracking asset(s)");
    }
}

/// archive-remote-content-batch processor.
pub fn archive_batch(state: &AppState, payload: &Value) -> Result<(), String> {
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
    // No pooled connection is held across the loop: each email's DB touches are
    // short scopes and its network fetches hold none.
    for email_id in unique {
        if let Err(e) = archive_email_remote_content(state, &email_id) {
            if let Ok(conn) = state.pool.get() {
                conn.execute(
                    "UPDATE archived_emails SET remote_content_status = 'failed', \
                     remote_content_archived_at = ? WHERE id = ?",
                    rusqlite::params![now_ms(), email_id],
                )
                .ok();
            }
            eprintln!("[remote-content] archive failed for {email_id}: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_reserved_ipv4_blocked() {
        for ip in [
            [10, 0, 0, 1],
            [127, 0, 0, 1],
            [192, 168, 1, 1],
            [172, 16, 0, 1],
            [169, 254, 1, 1],
            [100, 64, 0, 1],
            [0, 0, 0, 0],
        ] {
            assert!(is_private_ipv4(ip), "{ip:?} should be blocked");
        }
    }

    #[test]
    fn test_net_ranges_only_block_the_24() {
        assert!(is_private_ipv4([198, 51, 100, 5]));
        assert!(is_private_ipv4([203, 0, 113, 5]));
        // the rest of those /16s is public
        assert!(!is_private_ipv4([198, 51, 1, 1]));
        assert!(!is_private_ipv4([203, 0, 1, 1]));
        // 192.0.0.0/24 and TEST-NET-1 192.0.2.0/24 block; the rest of 192.0/16
        // and all of the (public, globally-routable) 192.2/16 must NOT.
        assert!(is_private_ipv4([192, 0, 0, 5]));
        assert!(is_private_ipv4([192, 0, 2, 5]));
        assert!(!is_private_ipv4([192, 0, 1, 5]));
        assert!(!is_private_ipv4([192, 0, 99, 5]));
        assert!(!is_private_ipv4([192, 2, 3, 4]));
    }

    #[test]
    fn public_ipv4_allowed() {
        assert!(!is_private_ipv4([8, 8, 8, 8]));
        assert!(!is_private_ipv4([1, 1, 1, 1]));
    }

    #[test]
    fn extension_for_types() {
        assert_eq!(extension_for("image/png"), ".png");
        assert_eq!(extension_for("image/jpeg"), ".jpg");
        assert_eq!(extension_for("image/webp"), ".webp");
        assert_eq!(extension_for("application/pdf"), ".bin");
    }

    #[test]
    fn http_status_reasons() {
        assert_eq!(
            describe_http_status(reqwest::StatusCode::NOT_FOUND),
            "Remote server returned HTTP 404 Not Found"
        );
        assert_eq!(
            describe_http_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
            "Remote server returned HTTP 500 Internal Server Error"
        );
        // A code with no canonical reason phrase omits it.
        assert_eq!(
            describe_http_status(reqwest::StatusCode::from_u16(599).unwrap()),
            "Remote server returned HTTP 599"
        );
    }

    #[test]
    fn validate_archivable_sniffs_and_rejects() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(validate_archivable(&png, None).ok().as_deref(), Some("image/png"));
        assert!(validate_archivable(b"", None).is_err());
        assert!(validate_archivable(b"not an image", Some("text/plain")).is_err());
        assert_eq!(
            validate_archivable(b"body { color: red }", Some("text/css")).ok().as_deref(),
            Some("text/css")
        );
        // The block reason names the actual, specific content type.
        let svg = validate_archivable(b"<svg></svg>", Some("image/svg+xml")).unwrap_err();
        assert!(svg.message().contains("image/svg+xml"), "reason: {}", svg.message());
        let html = validate_archivable(b"<html></html>", Some("text/html; charset=utf-8")).unwrap_err();
        assert!(html.message().contains("text/html"), "reason: {}", html.message());
    }

    #[test]
    fn ipv6_blocking() {
        use std::net::IpAddr;
        assert!(is_blocked_ip(&"::1".parse::<IpAddr>().unwrap()), "loopback");
        assert!(is_blocked_ip(&"::ffff:127.0.0.1".parse::<IpAddr>().unwrap()), "mapped loopback");
        assert!(is_blocked_ip(&"::127.0.0.1".parse::<IpAddr>().unwrap()), "ipv4-compatible loopback");
        assert!(is_blocked_ip(&"2002:7f00:0001::".parse::<IpAddr>().unwrap()), "6to4 wrapping 127.0.0.1");
        assert!(is_blocked_ip(&"64:ff9b::a00:1".parse::<IpAddr>().unwrap()), "NAT64 wrapping 10.0.0.1");
        assert!(!is_blocked_ip(&"2001:4860:4860::8888".parse::<IpAddr>().unwrap()), "public ipv6");
        assert!(!is_blocked_ip(&"2002:0808:0808::".parse::<IpAddr>().unwrap()), "6to4 wrapping public 8.8.8.8");
    }
}
