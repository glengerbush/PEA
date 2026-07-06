//! Port of RemoteContentService.buildPreview + remote-asset endpoints.
//!
//! The CSS sanitization, URL rewriting, cid inlining, and tracking heuristics
//! are exact ports of the Node code. The final HTML pass uses ammonia with the
//! same policy sanitize-html enforced (allowed tags/attributes/schemes,
//! link hardening, img rewriting) — the *serialized markup* can differ in
//! insignificant ways (attribute order, entity escaping), so the golden-diff
//! harness compares the preview HTML semantically, not byte-for-byte.

use crate::state::AppState;
use mail_parser::MimeHeaders;
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use base64::Engine as _;
use std::sync::LazyLock;
use regex::Regex;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

const MAX_INLINE_CID_BYTES: usize = 1024 * 1024;

const PREVIEW_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; \
img-src 'self' data: http://localhost:* http://127.0.0.1:* http://[::1]:* \
https://localhost:* https://127.0.0.1:* https://[::1]:*; \
style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'";

const SAFE_IMAGE_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];

static TRACKING_URL_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)/track/open",
        r"(?i)/wf/open",
        r"(?i)/(?:email|e)/open",
        r"(?i)/open\?",
        r"(?i)\bopen\.(?:aspx|php|gif|png|jpe?g)\b",
        r"(?i)/(?:o|oo|p|px|pixel|beacon)\.(?:gif|png|jpe?g)\b",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

pub fn normalize_content_type(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    let content_type = value.split(';').next().unwrap_or("").trim().to_lowercase();
    if content_type == "image/jpg" {
        return Some("image/jpeg".into());
    }
    if content_type.is_empty() {
        None
    } else {
        Some(content_type)
    }
}

pub fn is_safe_preview_content_type(content_type: Option<&str>) -> bool {
    content_type.map_or(false, |ct| SAFE_IMAGE_TYPES.contains(&ct))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Decodes the handful of HTML entities we care about in a URL/attribute value,
/// in a SINGLE pass — a '&' produced by decoding one entity is never re-scanned
/// as the start of another. (The old sequential replace_all double-decoded e.g.
/// `&amp;#38;` → `&` and `&#38;amp;` → `&`, so the archive and preview could key
/// the same asset differently.)
pub fn decode_html_attribute(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            // Bounded scan for the terminating ';' (longest handled form is
            // "#x10FFFF"); beyond that, treat '&' as a literal, not an entity.
            if let Some(rel) = value[i + 1..].find(';').filter(|r| *r <= 12) {
                if let Some(decoded) = decode_html_entity(&value[i + 1..i + 1 + rel]) {
                    out.push_str(&decoded);
                    i += 1 + rel + 1;
                    continue;
                }
            }
        }
        // Not an entity — copy one full UTF-8 char.
        let ch = value[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Decodes a single entity body (the text between `&` and `;`): the named set the
/// preview supports plus numeric `#NN` / `#xNN`. Returns None for anything else,
/// so an unrecognised entity is left verbatim.
fn decode_html_entity(name: &str) -> Option<String> {
    match name.to_ascii_lowercase().as_str() {
        "amp" => return Some("&".into()),
        "quot" => return Some("\"".into()),
        "apos" => return Some("'".into()),
        "lt" => return Some("<".into()),
        "gt" => return Some(">".into()),
        _ => {}
    }
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from);
    }
    if let Some(dec) = name.strip_prefix('#') {
        return dec.parse::<u32>().ok().and_then(char::from_u32).map(String::from);
    }
    None
}

pub fn to_remote_url(value: &str) -> Option<String> {
    static CTRL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\x00-\x1f\x7f]+").unwrap());
    let trimmed = CTRL.replace_all(value, "");
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return None;
    }
    let url = url::Url::parse(trimmed).ok()?;
    match url.scheme() {
        "http" | "https" => Some(url.to_string()),
        _ => None,
    }
}

fn extract_srcset_urls(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|item| item.trim().split_whitespace().next().map(String::from))
        .filter_map(|u| to_remote_url(&u))
        .collect()
}

fn extract_css_urls(value: &str) -> Vec<String> {
    static URL_PAT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)url\(\s*(?:"([^"]+)"|'([^']+)'|([^)]+))\s*\)"#).unwrap()
    });
    URL_PAT
        .captures_iter(value)
        .filter_map(|m| {
            let raw = m
                .get(1)
                .or_else(|| m.get(2))
                .or_else(|| m.get(3))
                .map(|g| g.as_str())
                .unwrap_or("");
            to_remote_url(&decode_html_attribute(raw))
        })
        .collect()
}

/// url(...) with optionally matching quotes — the JS regex uses a backreference,
/// which the regex crate lacks, so quoted/unquoted forms are separate branches.
static CSS_URL_REWRITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)url\(\s*'([^'")]*)'\s*\)|url\(\s*"([^'")]*)"\s*\)|url\(\s*([^'")]*)\s*\)"#)
        .unwrap()
});

fn rewrite_css_urls(css: &str, rewrite_url: &dyn Fn(&str) -> Option<String>) -> String {
    CSS_URL_REWRITE
        .replace_all(css, |c: &regex::Captures| {
            let raw = c
                .get(1)
                .or_else(|| c.get(2))
                .or_else(|| c.get(3))
                .map(|g| g.as_str())
                .unwrap_or("");
            let rewritten = rewrite_url(&decode_html_attribute(raw.trim()));
            format!("url('{}')", rewritten.unwrap_or_else(|| "data:,".into()))
        })
        .into_owned()
}

fn sanitize_css_text(css: &str, rewrite_url: &dyn Fn(&str) -> Option<String>) -> String {
    static COMMENTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").unwrap());
    static IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)@import\b[^;]*;?").unwrap());
    static EXPRESSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)expression\s*\([^)]*\)").unwrap());
    static BEHAVIOR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(?:behavior|-moz-binding)\s*:[^;}]*").unwrap());
    static JS_PROTO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)javascript:").unwrap());
    // image-set()/cross-fade() take bare string URLs that rewrite_css_urls
    // (url()-only) never sees, so a crafted email could fetch from loopback.
    static IMAGE_FN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:-webkit-)?(?:image-set|cross-fade)\s*\([^;}]*\)").unwrap()
    });
    let cleaned = COMMENTS.replace_all(css, "");
    let cleaned = IMPORT.replace_all(&cleaned, "");
    let cleaned = EXPRESSION.replace_all(&cleaned, "");
    let cleaned = BEHAVIOR.replace_all(&cleaned, "");
    let cleaned = JS_PROTO.replace_all(&cleaned, "");
    let cleaned = IMAGE_FN.replace_all(&cleaned, "none");
    let cleaned = cleaned.replace(['<', '>'], "");
    rewrite_css_urls(&cleaned, rewrite_url).trim().to_string()
}

fn sanitize_style(value: &str, rewrite_url: &dyn Fn(&str) -> Option<String>) -> String {
    static IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)@import\b[^;]*;?").unwrap());
    static EXPRESSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)expression\s*\([^)]*\)").unwrap());
    static BEHAVIOR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(?:behavior|-moz-binding)\s*:[^;]*").unwrap());
    static JS_PROTO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)javascript:").unwrap());
    static IMAGE_FN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:-webkit-)?(?:image-set|cross-fade)\s*\([^;}]*\)").unwrap()
    });
    let cleaned = IMPORT.replace_all(value, "");
    let cleaned = EXPRESSION.replace_all(&cleaned, "");
    let cleaned = BEHAVIOR.replace_all(&cleaned, "");
    let cleaned = JS_PROTO.replace_all(&cleaned, "");
    let cleaned = IMAGE_FN.replace_all(&cleaned, "none");
    cleaned
        .split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| rewrite_css_urls(d, rewrite_url))
        .collect::<Vec<_>>()
        .join("; ")
}

fn is_safe_link_url(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return true;
    }
    match url::Url::parse(trimmed) {
        Ok(u) => matches!(u.scheme(), "http" | "https" | "mailto" | "tel"),
        Err(_) => false,
    }
}

/// A remote-content asset row, as needed by the preview/list endpoints.
#[derive(Clone)]
pub struct AssetRecord {
    pub id: String,
    pub original_url: String,
    pub final_url: Option<String>,
    pub status: String,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub storage_path: Option<String>,
    pub failure_reason: Option<String>,
}

fn load_assets(conn: &rusqlite::Connection, email_id: &str) -> Vec<AssetRecord> {
    let mut stmt = conn
        .prepare(
            "SELECT id, original_url, final_url, status, content_type, size_bytes, \
             storage_path, failure_reason FROM remote_content_assets WHERE email_id = ?",
        )
        .unwrap();
    stmt.query_map([email_id], |row| {
        Ok(AssetRecord {
            id: row.get(0)?,
            original_url: row.get(1)?,
            final_url: row.get(2)?,
            status: row.get(3)?,
            content_type: row.get(4)?,
            size_bytes: row.get(5)?,
            storage_path: row.get(6)?,
            failure_reason: row.get(7)?,
        })
    })
    .unwrap()
    .filter_map(Result::ok)
    .collect()
}

fn rewrite_image_source(
    email_id: &str,
    value: &str,
    cid_map: &HashMap<String, String>,
    asset_by_url: &HashMap<String, AssetRecord>,
) -> Option<String> {
    let trimmed = value.trim();
    static CID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^cid:(.+)$").unwrap());
    if let Some(m) = CID.captures(trimmed) {
        // Lowercase to match how cid_map keys and the referenced set are
        // normalized — cid:Logo must resolve a `Content-ID: <logo>` part.
        let cid = m[1].trim_start_matches('<').trim_end_matches('>').to_lowercase();
        return cid_map.get(&cid).cloned();
    }
    static DATA_IMG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^data:image/(png|jpeg|gif|webp|avif);base64,").unwrap());
    if DATA_IMG.is_match(trimmed) {
        return Some(trimmed.to_string());
    }
    let remote_url = to_remote_url(trimmed)?;
    let asset = asset_by_url.get(&remote_url)?;
    if asset.status != "archived"
        || asset.storage_path.is_none()
        || !is_safe_preview_content_type(normalize_content_type(asset.content_type.as_deref()).as_deref())
    {
        return None;
    }
    Some(format!(
        "/api/v1/archived-emails/{email_id}/remote-assets/{}",
        asset.id
    ))
}

fn safe_srcset_descriptor(value: &str) -> bool {
    static W: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+w$").unwrap());
    static X: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(\.\d+)?x$").unwrap());
    W.is_match(value) || X.is_match(value)
}

fn rewrite_srcset(
    email_id: &str,
    value: &str,
    cid_map: &HashMap<String, String>,
    asset_by_url: &HashMap<String, AssetRecord>,
) -> Option<String> {
    let entries: Vec<String> = value
        .split(',')
        .filter_map(|entry| {
            let mut parts = entry.trim().split_whitespace();
            let raw_url = parts.next().unwrap_or("");
            let rewritten = rewrite_image_source(email_id, raw_url, cid_map, asset_by_url)?;
            let mut out = vec![rewritten];
            out.extend(parts.filter(|d| safe_srcset_descriptor(d)).map(String::from));
            Some(out.join(" "))
        })
        .collect();
    if entries.is_empty() {
        None
    } else {
        Some(entries.join(", "))
    }
}

fn get_tag_attribute(tag: &str, name: &str) -> Option<String> {
    let pattern = format!(
        r#"(?i)\b{name}\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))"#
    );
    let re = Regex::new(&pattern).ok()?;
    let m = re.captures(tag)?;
    m.get(2)
        .or_else(|| m.get(3))
        .or_else(|| m.get(4))
        .map(|g| g.as_str().to_string())
}

fn get_attribute_map(raw_attributes: &str) -> HashMap<String, String> {
    static ATTR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"([^\s=/"'<>`]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?"#).unwrap()
    });
    let mut attributes = HashMap::new();
    for m in ATTR.captures_iter(raw_attributes) {
        let name = m[1].to_lowercase();
        let value = m
            .get(2)
            .or_else(|| m.get(3))
            .or_else(|| m.get(4))
            .map(|g| g.as_str())
            .unwrap_or("");
        // JS Map.set: last occurrence wins — insert unconditionally.
        attributes.insert(name, decode_html_attribute(value));
    }
    attributes
}

fn parse_pixel_dimension(value: Option<&str>) -> Option<f64> {
    static DIM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+(?:\.\d+)?)(?:px)?$").unwrap());
    let value = value?;
    let lowered = value.trim().to_lowercase();
    let m = DIM.captures(&lowered)?;
    m[1].parse().ok()
}

fn get_inline_style_property(style: &str, property: &str) -> Option<String> {
    let re = Regex::new(&format!(r"(?i)(?:^|;)\s*{property}\s*:\s*([^;]+)")).ok()?;
    re.captures(style).map(|m| m[1].trim().to_string())
}

fn is_likely_tracking_pixel(attributes: &HashMap<String, String>) -> bool {
    static HIDDEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|;)\s*display\s*:\s*none").unwrap());
    static INVISIBLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|;)\s*visibility\s*:\s*hidden").unwrap());
    let style = attributes.get("style").cloned().unwrap_or_default().to_lowercase();
    if HIDDEN.is_match(&style) || INVISIBLE.is_match(&style) {
        return true;
    }
    let width = parse_pixel_dimension(
        get_inline_style_property(&style, "width")
            .or_else(|| attributes.get("width").cloned())
            .as_deref(),
    );
    let height = parse_pixel_dimension(
        get_inline_style_property(&style, "height")
            .or_else(|| attributes.get("height").cloned())
            .as_deref(),
    );
    matches!((width, height), (Some(w), Some(h)) if w <= 2.0 && h <= 2.0)
}

/// Hosts that exist ONLY to serve tracking / analytics beacons — never real email
/// images — so ANY URL on them is a tracker. Suffix-matched (exact host or a
/// subdomain), which keeps false positives at zero for real content. Curated for
/// the largest analytics/ad-tech providers; add more here as they're identified.
const TRACKING_HOSTS: [&str; 9] = [
    "google-analytics.com",  // Google Analytics collection
    "emltrk.com",            // Litmus email open tracking
    "2o7.net",               // Adobe Analytics (Omniture)
    "omtrdc.net",            // Adobe Experience Cloud collection
    "scorecardresearch.com", // Comscore
    "quantserve.com",        // Quantcast
    "px.ads.linkedin.com",   // LinkedIn Insight pixel
    "track.hubspot.com",     // HubSpot open tracking
    "analytics.twitter.com", // X / Twitter analytics
];

/// True when `host` equals `base` or is a subdomain of it (`x.base`).
fn host_matches(host: &str, base: &str) -> bool {
    host == base
        || (host.len() > base.len()
            && host.ends_with(base)
            && host.as_bytes()[host.len() - base.len() - 1] == b'.')
}

/// Amazon storefront host (amazon.<tld>, incl. www./regional subdomains) — NOT the
/// image CDNs like ssl-images-amazon.com, so "amazon" must be a full domain label.
fn is_amazon_storefront_host(host: &str) -> bool {
    static AMAZON: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(?:^|\.)amazon\.[a-z]{2,}(?:\.[a-z]{2,})?$").unwrap());
    AMAZON.is_match(host)
}

/// Domain-gated tracker detection: dedicated whole-host trackers, plus specific
/// tracking endpoints on hosts that also serve real content (path-gated so a real
/// image from those providers is never skipped).
fn is_known_domain_tracker(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if TRACKING_HOSTS.iter().any(|base| host_matches(&host, base)) {
        return true;
    }
    let path = parsed.path();
    // Amazon marketing open-tracker redirect (resolves to a transparent pixel):
    //   amazon.<tld>/gp/r.html?...&U=<pixel>
    if is_amazon_storefront_host(&host) && path.eq_ignore_ascii_case("/gp/r.html") {
        return true;
    }
    // Meta (Facebook) tracking pixel: facebook.com/tr
    if host_matches(&host, "facebook.com")
        && (path.eq_ignore_ascii_case("/tr") || path.eq_ignore_ascii_case("/tr.php"))
    {
        return true;
    }
    false
}

fn is_likely_tracking_url(url: &str) -> bool {
    TRACKING_URL_PATTERNS.iter().any(|p| p.is_match(url)) || is_known_domain_tracker(url)
}

pub fn is_likely_tracking_url_pub(url: &str) -> bool {
    is_likely_tracking_url(url)
}

/// Port of extractRemoteUrls — unique non-tracking remote URLs in JS-Set
/// insertion order (the archive pipeline fetches them in this order).
pub fn extract_remote_urls_ordered(html: &str) -> Vec<String> {
    let set = extract_remote_urls(html);
    set.ordered
}

struct OrderedSet {
    seen: HashSet<String>,
    ordered: Vec<String>,
}

impl OrderedSet {
    fn insert(&mut self, value: String) {
        if self.seen.insert(value.clone()) {
            self.ordered.push(value);
        }
    }
    fn len(&self) -> usize {
        self.ordered.len()
    }
}

fn extract_remote_urls(html: &str) -> OrderedSet {
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([a-zA-Z][\w:-]*)([^>]*)>").unwrap());
    let mut urls = OrderedSet { seen: HashSet::new(), ordered: Vec::new() };
    let add_url = |url: Option<String>, urls: &mut OrderedSet| {
        if let Some(u) = url {
            if !is_likely_tracking_url(&u) {
                urls.insert(u);
            }
        }
    };
    for m in TAG.captures_iter(html) {
        let tag = m[1].to_lowercase();
        let attrs = get_attribute_map(&m[2]);
        let is_tracking_pixel = is_likely_tracking_pixel(&attrs);
        if !is_tracking_pixel {
            for attr_name in ["src", "background", "poster"] {
                if let Some(value) = attrs.get(attr_name) {
                    add_url(to_remote_url(value), &mut urls);
                }
            }
            if let Some(srcset) = attrs.get("srcset") {
                for url in extract_srcset_urls(srcset) {
                    add_url(Some(url), &mut urls);
                }
            }
        }
        if let Some(style) = attrs.get("style") {
            for url in extract_css_urls(style) {
                add_url(Some(url), &mut urls);
            }
        }
        if tag == "link" {
            if let Some(href) = attrs.get("href") {
                add_url(to_remote_url(href), &mut urls);
            }
        }
    }
    for url in extract_css_urls(html) {
        add_url(Some(url), &mut urls);
    }
    urls
}

fn render_text_preview(text: &str) -> String {
    static NEWLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r?\n").unwrap());
    format!("<div>{}</div>", NEWLINE.replace_all(&escape_html(text), "<br>"))
}

/// Resolves a part's bytes: directly from the message, or — for hollowed
/// parts (empty body + X-PEA-Attachment marker) — from the blob store.
type PartResolver<'a> = dyn Fn(&mail_parser::MessagePart) -> Option<Vec<u8>> + 'a;

/// Inline-disposition images that no body references by CID (Apple Mail —
/// especially iPhone — sends body photos this way, with no CID at all).
/// Returns `(mime part id, <img> markup)` so callers can interleave them
/// with the text parts in original MIME order.
fn inline_images_without_reference(
    msg: &mail_parser::Message,
    resolve: &PartResolver,
) -> Vec<(usize, String)> {
    let raw_html: String = msg
        .html_body
        .iter()
        .filter_map(|&id| msg.part(id).and_then(|p| p.text_contents()))
        .collect::<Vec<_>>()
        .join("\n");
    let referenced = crate::ingest::extract_cid_references(&raw_html);

    let mut images = Vec::new();
    for &id in msg.attachments.iter() {
        let Some(part) = msg.part(id) else { continue };
        let disposition = part
            .content_disposition()
            .map(|d| d.ctype().to_lowercase());
        let content_type = normalize_content_type(crate::ingest::part_content_type(part).as_deref());
        let is_image = content_type
            .as_deref()
            .map_or(false, |ct| ct.starts_with("image/"));
        match disposition.as_deref() {
            Some("inline") => {}
            None if is_image => {}
            _ => continue,
        }
        if !is_safe_preview_content_type(content_type.as_deref()) {
            continue;
        }
        if let Some(cid) = part.content_id() {
            let cid = cid.trim_start_matches('<').trim_end_matches('>').to_lowercase();
            if referenced.contains(&cid) {
                continue;
            }
        }
        let Some(contents) = resolve(part) else { continue };
        if contents.is_empty() || contents.len() > MAX_INLINE_CID_BYTES {
            continue;
        }
        let alt = part.attachment_name().unwrap_or("");
        images.push((
            id,
            format!(
                "<div><img src=\"data:{};base64,{}\" alt=\"{}\" style=\"max-width: 100%\"></div>",
                content_type.unwrap(),
                base64::engine::general_purpose::STANDARD.encode(&contents),
                escape_html(alt)
            ),
        ));
    }
    images
}

/// Builds the unsanitized preview body: the html body when present, else the
/// text parts — with unreferenced inline images rendered in place.
fn build_preview_body(msg: &mail_parser::Message, resolve: &PartResolver) -> String {
    let (text_part, html_part) = crate::ingest::mailparser_text_and_html(msg);
    let images = inline_images_without_reference(msg, resolve);
    if images.is_empty() {
        return if !html_part.trim().is_empty() {
            html_part
        } else {
            render_text_preview(&text_part)
        };
    }
    if !html_part.trim().is_empty() {
        let imgs: String = images.into_iter().map(|(_, tag)| tag).collect();
        return format!("{html_part}{imgs}");
    }
    // No html body: interleave text parts and images in MIME order.
    let mut segments = images;
    for &id in msg.text_body.iter() {
        let Some(part) = msg.part(id) else { continue };
        if crate::ingest::part_content_type(part).as_deref() == Some("text/html") {
            continue;
        }
        let Some(text) = part.text_contents() else { continue };
        if text.trim().is_empty() {
            continue;
        }
        segments.push((id, render_text_preview(text)));
    }
    segments.sort_by_key(|(id, _)| *id);
    segments
        .into_iter()
        .map(|(_, markup)| markup)
        .collect::<Vec<_>>()
        .join("")
}

const ALLOWED_TAGS: [&str; 43] = [
    "a", "abbr", "b", "big", "blockquote", "br", "caption", "center", "cite", "code", "col",
    "colgroup", "dd", "del", "div", "dl", "dt", "em", "font", "h1", "h2", "h3", "h4", "h5", "h6",
    "hr", "i", "img", "li", "ol", "p", "pre", "s", "small", "span", "strong", "sub", "sup",
    "table", "tbody", "td", "tfoot", "th",
];
const ALLOWED_TAGS_2: [&str; 4] = ["thead", "tr", "u", "ul"];

const SAFE_GLOBAL_ATTRIBUTES: [&str; 16] = [
    "align", "alt", "bgcolor", "border", "cellpadding", "cellspacing", "class", "colspan", "dir",
    "height", "lang", "rowspan", "style", "title", "valign", "width",
];

/// Port of sanitizeEmailPreviewHtml. CSS handling is an exact port; the HTML
/// pass runs ammonia under the equivalent policy.
fn sanitize_email_preview_html(
    email_id: &str,
    html: &str,
    cid_map: &HashMap<String, String>,
    assets: &[AssetRecord],
    css_by_url: &HashMap<String, String>,
) -> String {
    let mut asset_by_url: HashMap<String, AssetRecord> = HashMap::new();
    for asset in assets {
        asset_by_url.insert(asset.original_url.clone(), asset.clone());
        if let Some(final_url) = &asset.final_url {
            asset_by_url.insert(final_url.clone(), asset.clone());
        }
    }

    let rewrite_url =
        |raw: &str| rewrite_image_source(email_id, raw, cid_map, &asset_by_url);

    let mut style_blocks: Vec<String> = Vec::new();

    // Inline archived external stylesheets referenced by <link rel="stylesheet">.
    if !css_by_url.is_empty() {
        static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<link\b[^>]*>").unwrap());
        for m in LINK.find_iter(html) {
            let tag = m.as_str();
            let rel = get_tag_attribute(tag, "rel").unwrap_or_default().to_lowercase();
            if !rel.split_whitespace().any(|r| r == "stylesheet") {
                continue;
            }
            let href = get_tag_attribute(tag, "href")
                .and_then(|h| to_remote_url(&decode_html_attribute(&h)));
            let Some(href) = href else { continue };
            let Some(css) = css_by_url.get(&href) else { continue };
            let sheet_rewrite = |raw: &str| -> Option<String> {
                // Resolve sub-resource URLs against the stylesheet's FINAL url
                // (post-redirect), matching the keys archive_stylesheet_subresources
                // stored them under; fall back to the link href when none recorded.
                let base = asset_by_url
                    .get(&href)
                    .and_then(|a| a.final_url.clone())
                    .unwrap_or_else(|| href.clone());
                let absolute = url::Url::parse(&base)
                    .ok()
                    .and_then(|b| b.join(raw).ok())
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| raw.to_string());
                rewrite_image_source(email_id, &absolute, cid_map, &asset_by_url)
            };
            let safe = sanitize_css_text(css, &sheet_rewrite);
            if !safe.is_empty() {
                style_blocks.push(safe);
            }
        }
    }

    // Pull the email's own <style> blocks out (sanitized) before the HTML pass.
    // Match to the closing tag OR end of input, so an unclosed <style> block is
    // still extracted/sanitized instead of leaking its raw CSS as visible text.
    static STYLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<style\b[^>]*>(.*?)(?:</style\s*>|$)").unwrap());
    let html_without_style_tags = STYLE.replace_all(html, |c: &regex::Captures| {
        let safe = sanitize_css_text(&c[1], &rewrite_url);
        if !safe.is_empty() {
            style_blocks.push(safe);
        }
        String::new()
    });

    let tags: HashSet<&str> = ALLOWED_TAGS.iter().chain(ALLOWED_TAGS_2.iter()).copied().collect();
    let generic: HashSet<&str> = SAFE_GLOBAL_ATTRIBUTES.iter().copied().collect();
    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attrs.insert("a", ["href", "target"].into_iter().collect());
    tag_attrs.insert("img", ["src", "srcset"].into_iter().collect());

    // ammonia's attribute_filter must be 'static — move owned copies in.
    let email_id_owned = email_id.to_string();
    let cid_map_owned = cid_map.clone();
    let asset_by_url_owned = asset_by_url.clone();
    let sanitized_body = ammonia::Builder::default()
        .tags(tags)
        .generic_attributes(generic)
        .tag_attributes(tag_attrs)
        .url_schemes(
            ["http", "https", "mailto", "tel", "data"]
                .into_iter()
                .collect(),
        )
        .url_relative(ammonia::UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .attribute_filter(move |element, attribute, value| -> Option<Cow<str>> {
            let rewrite = |raw: &str| {
                rewrite_image_source(&email_id_owned, raw, &cid_map_owned, &asset_by_url_owned)
            };
            if attribute == "style" {
                let safe = sanitize_style(value, &rewrite);
                return if safe.is_empty() { None } else { Some(safe.into()) };
            }
            match (element, attribute) {
                ("a", "href") => {
                    if is_safe_link_url(value) {
                        Some(value.to_string().into())
                    } else {
                        None
                    }
                }
                ("img", "src") => rewrite(value).map(Cow::from),
                ("img", "srcset") => {
                    rewrite_srcset(&email_id_owned, value, &cid_map_owned, &asset_by_url_owned)
                        .map(Cow::from)
                }
                _ => Some(value.to_string().into()),
            }
        })
        .clean(&html_without_style_tags)
        .to_string();

    // exclusiveFilter: drop <img> that lost both src and srcset. Parse real
    // attributes (not a substring match) so an alt/title value containing the
    // text "src=" can't keep a source-less, broken-image tag alive.
    static EMPTY_IMG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<img\b[^>]*>").unwrap());
    let sanitized_body = EMPTY_IMG.replace_all(&sanitized_body, |c: &regex::Captures| {
        let attrs = get_attribute_map(&c[0]);
        if attrs.contains_key("src") || attrs.contains_key("srcset") {
            c[0].to_string()
        } else {
            String::new()
        }
    });

    let style_tag = if style_blocks.is_empty() {
        String::new()
    } else {
        format!("<style>{}</style>", style_blocks.join("\n"))
    };
    format!("{style_tag}{sanitized_body}")
}

fn not_found(message: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "message": message }))).into_response()
}

struct EmailForPreview {
    storage_path: String,
    remote_content_status: String,
}

fn get_email_for_preview(conn: &rusqlite::Connection, email_id: &str) -> Option<EmailForPreview> {
    conn.query_row(
        "SELECT storage_path, remote_content_status FROM archived_emails WHERE id = ?",
        [email_id],
        |row| {
            Ok(EmailForPreview {
                storage_path: row.get(0)?,
                remote_content_status: row.get(1)?,
            })
        },
    )
    .ok()
}

fn read_storage(app: &AppState, path: &str) -> Option<Vec<u8>> {
    app.storage_abs(path).ok().and_then(|file| std::fs::read(file).ok())
}

// ---------------------------------------------------------------------------
// GET /archived-emails/:id/preview
// ---------------------------------------------------------------------------

pub async fn email_preview(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = app.pool.get().unwrap();
    let Some(email) = get_email_for_preview(&conn, &id) else {
        return not_found("Archived email not found");
    };
    let Some(raw) = read_storage(&app, &email.storage_path) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "message": "An internal server error occurred" })),
        )
            .into_response();
    };
    let message = mail_parser::MessageParser::default().parse(&raw);
    let assets = load_assets(&conn, &id);

    // Blob-store paths for this email's (hollowed) attachments, by sha-256.
    let mut blob_paths: HashMap<String, (String, i64)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT a.content_hash_sha256, a.storage_path, a.size_bytes \
                 FROM email_attachments ea \
                 INNER JOIN attachments a ON ea.attachment_id = a.id \
                 WHERE ea.email_id = ?",
            )
            .unwrap();
        let rows = stmt
            .query_map([&id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .unwrap()
            .filter_map(Result::ok);
        for (hash, path, size) in rows {
            blob_paths.insert(hash, (path, size));
        }
    }
    let resolve = |part: &mail_parser::MessagePart| -> Option<Vec<u8>> {
        let contents = part.contents();
        if !contents.is_empty() {
            return Some(contents.to_vec());
        }
        let hash = crate::ingest::part_pea_marker(part)?;
        let (path, size) = blob_paths.get(&hash)?;
        if *size > MAX_INLINE_CID_BYTES as i64 {
            return None;
        }
        read_storage(&app, path)
    };

    // Same text/html semantics as mailparser (shared with the ingest pipeline);
    // parsedEmail.html already has cid: images replaced with data: URIs.
    let html_source = match &message {
        Some(msg) => crate::ingest::mailparser_text_and_html(msg).1,
        None => String::new(),
    };
    let html = match &message {
        Some(msg) => build_preview_body(msg, &resolve),
        None => String::new(),
    };

    // cid → data-URI map from safely-typed inline attachments (≤ 1 MiB).
    let mut cid_map: HashMap<String, String> = HashMap::new();
    if let Some(msg) = &message {
        for attachment in msg.attachments() {
            let Some(cid) = attachment.content_id() else { continue };
            let content_type = attachment.content_type().map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_string(),
            });
            let normalized = normalize_content_type(content_type.as_deref());
            if !is_safe_preview_content_type(normalized.as_deref()) {
                continue;
            }
            let Some(contents) = resolve(attachment) else { continue };
            if contents.is_empty() || contents.len() > MAX_INLINE_CID_BYTES {
                continue;
            }
            let cid = cid.trim_start_matches('<').trim_end_matches('>').to_lowercase();
            cid_map.insert(
                cid,
                format!(
                    "data:{};base64,{}",
                    normalized.unwrap(),
                    base64::engine::general_purpose::STANDARD.encode(&contents)
                ),
            );
        }
    }

    // Archived text/css assets, keyed by original + final URL, for inlining.
    let mut css_by_url: HashMap<String, String> = HashMap::new();
    for asset in &assets {
        if asset.status != "archived" {
            continue;
        }
        let Some(path) = &asset.storage_path else { continue };
        if normalize_content_type(asset.content_type.as_deref()).as_deref() != Some("text/css") {
            continue;
        }
        if let Some(bytes) = read_storage(&app, path) {
            let text = String::from_utf8_lossy(&bytes).to_string();
            css_by_url.insert(asset.original_url.clone(), text.clone());
            if let Some(final_url) = &asset.final_url {
                css_by_url.insert(final_url.clone(), text);
            }
        }
    }

    let safe_html = sanitize_email_preview_html(&id, &html, &cid_map, &assets, &css_by_url);
    let remote_urls = extract_remote_urls(&html_source);
    let count = |status: &str| assets.iter().filter(|a| a.status == status).count();

    Json(json!({
        "emailId": id,
        "html": format!(
            "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"{PREVIEW_CONTENT_SECURITY_POLICY}\"><base target=\"_blank\"></head><body>{safe_html}</body></html>"
        ),
        "status": email.remote_content_status,
        "remoteUrlCount": remote_urls.len(),
        "archivedAssetCount": count("archived"),
        "blockedAssetCount": count("blocked"),
        "failedAssetCount": count("failed"),
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /archived-emails/:id/remote-assets
// ---------------------------------------------------------------------------

pub async fn list_remote_assets(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = app.pool.get().unwrap();
    if get_email_for_preview(&conn, &id).is_none() {
        return not_found("Archived email not found");
    }
    let mut assets = load_assets(&conn, &id);
    assets.retain(|a| a.status != "pending");
    assets.retain(|a| {
        !(a.status == "archived"
            && normalize_content_type(a.content_type.as_deref()).as_deref() == Some("text/css"))
    });
    let rank = |status: &str| match status {
        "archived" => 0,
        "failed" => 1,
        "blocked" => 2,
        "pending" => 3,
        _ => 9,
    };
    assets.sort_by_key(|a| rank(&a.status)); // stable, like JS Array.sort
    let out: Vec<Value> = assets
        .iter()
        .map(|a| {
            let content_type = normalize_content_type(a.content_type.as_deref());
            json!({
                "id": a.id,
                "originalUrl": a.original_url,
                "contentType": content_type,
                "sizeBytes": a.size_bytes,
                "status": a.status,
                "failureReason": a.failure_reason,
                "previewable": a.status == "archived"
                    && a.storage_path.is_some()
                    && is_safe_preview_content_type(content_type.as_deref()),
            })
        })
        .collect();
    Json(Value::Array(out)).into_response()
}

// ---------------------------------------------------------------------------
// GET /archived-emails/:id/remote-assets/:assetId
// ---------------------------------------------------------------------------

pub async fn get_remote_asset(
    State(app): State<AppState>,
    AxumPath((id, asset_id)): AxumPath<(String, String)>,
) -> Response {
    let conn = app.pool.get().unwrap();
    if get_email_for_preview(&conn, &id).is_none() {
        return not_found("Archived email not found");
    }
    let asset = load_assets(&conn, &id)
        .into_iter()
        .find(|a| a.id == asset_id);
    let Some(asset) = asset else {
        return not_found("Remote content asset not found");
    };
    if asset.status != "archived" || asset.storage_path.is_none() {
        return not_found("Remote content asset not found");
    }
    let content_type = normalize_content_type(asset.content_type.as_deref());
    if !is_safe_preview_content_type(content_type.as_deref()) {
        return not_found("Remote content asset is not previewable");
    }
    let Some(bytes) = read_storage(&app, asset.storage_path.as_deref().unwrap()) else {
        return not_found("An internal server error occurred");
    };
    let mut headers = HeaderMap::new();
    let ct = content_type.unwrap_or_else(|| "application/octet-stream".into());
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&ct).unwrap());
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; img-src 'self' data:"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    (headers, bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_content_type_cases() {
        assert_eq!(normalize_content_type(Some("image/jpg")).as_deref(), Some("image/jpeg"));
        assert_eq!(normalize_content_type(Some("image/PNG; charset=x")).as_deref(), Some("image/png"));
        assert_eq!(normalize_content_type(Some("")), None);
        assert_eq!(normalize_content_type(None), None);
    }

    #[test]
    fn safe_preview_content_types() {
        assert!(is_safe_preview_content_type(Some("image/png")));
        assert!(!is_safe_preview_content_type(Some("text/html")));
        assert!(!is_safe_preview_content_type(None));
    }

    #[test]
    fn escape_and_decode_html() {
        assert_eq!(escape_html(r#"<a href="x">&'"#), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
        assert_eq!(decode_html_attribute("&amp;&lt;&gt;&#65;&#x42;"), "&<>AB");
        // Single-pass: a '&' produced by decoding must NOT start another entity.
        assert_eq!(decode_html_attribute("&amp;#38;"), "&#38;");
        assert_eq!(decode_html_attribute("&#38;amp;"), "&amp;");
        assert_eq!(decode_html_attribute("&amp;lt;"), "&lt;");
        // Unrecognised or malformed entities are left verbatim.
        assert_eq!(decode_html_attribute("a&nbsp;b"), "a&nbsp;b");
        assert_eq!(decode_html_attribute("100% & more"), "100% & more");
        assert_eq!(decode_html_attribute("x&y"), "x&y");
    }

    #[test]
    fn known_domain_trackers_are_filtered() {
        // Amazon redirect open-tracker (the reported case) + a regional TLD.
        assert!(is_likely_tracking_url(
            "http://www.amazon.com/gp/r.html?R=X&C=Y&H=Z&T=E&U=http%3A%2F%2Fimages.amazon.com%2Fimages%2FG%2F01%2Fnav%2Ftransp.gif"
        ));
        assert!(is_likely_tracking_url("https://amazon.co.uk/gp/r.html?U=x"));
        // Dedicated whole-host trackers.
        assert!(is_likely_tracking_url("https://www.google-analytics.com/collect?v=1"));
        assert!(is_likely_tracking_url("https://region1.google-analytics.com/g/collect"));
        assert!(is_likely_tracking_url("https://sb.scorecardresearch.com/b?c1=2"));
        assert!(is_likely_tracking_url("https://px.ads.linkedin.com/collect?pid=1"));
        // Meta pixel (path-gated).
        assert!(is_likely_tracking_url("https://www.facebook.com/tr?id=123&ev=Open"));

        // NOT filtered: real images, and the tracker path on an unrelated host.
        assert!(!is_likely_tracking_url("https://images-na.ssl-images-amazon.com/images/I/abc.jpg"));
        assert!(!is_likely_tracking_url("https://example.com/gp/r.html?U=x"));
        assert!(!is_likely_tracking_url("https://www.facebook.com/logo.png"));
        assert!(!is_likely_tracking_url("https://notgoogle-analytics.com/hero.jpg"));
        assert!(!is_likely_tracking_url("https://cdn.example.com/hero.jpg"));
    }

    #[test]
    fn to_remote_url_only_http() {
        assert_eq!(to_remote_url("  http://x.com/a  ").as_deref(), Some("http://x.com/a"));
        assert!(to_remote_url("https://y.com").is_some());
        assert!(to_remote_url("javascript:alert(1)").is_none());
        assert!(to_remote_url("cid:logo").is_none());
        assert!(to_remote_url("").is_none());
    }

    #[test]
    fn is_safe_link_url_schemes() {
        assert!(is_safe_link_url("#anchor"));
        assert!(is_safe_link_url("https://x.com"));
        assert!(is_safe_link_url("mailto:a@x.com"));
        assert!(!is_safe_link_url("javascript:alert(1)"));
        assert!(!is_safe_link_url("data:text/html,x"));
    }

    #[test]
    fn sanitize_css_strips_dangerous_and_image_fns() {
        let drop = |_: &str| None;
        let out = sanitize_css_text(
            "a{behavior:url(x)} b{background:expression(alert(1))} @import 'z'; c{background:image-set('cid:hero')}",
            &drop,
        );
        assert!(!out.contains("behavior"));
        assert!(!out.contains("expression"));
        assert!(!out.contains("@import"));
        assert!(!out.contains("image-set"));
    }

    #[test]
    fn sanitize_style_strips_js_and_image_set() {
        let keep = |u: &str| Some(u.to_string());
        let out = sanitize_style("color:red; background:image-set('cid:x'); width:javascript:x", &keep);
        assert!(!out.contains("image-set"));
        assert!(!out.contains("javascript:"));
        assert!(out.contains("color:red"));
    }

    #[test]
    fn extract_srcset_and_css_urls() {
        assert_eq!(
            extract_srcset_urls("http://a.com/1.png 1x, http://b.com/2.png 2x"),
            vec!["http://a.com/1.png", "http://b.com/2.png"]
        );
        assert_eq!(
            extract_css_urls("background: url('http://c.com/x.png'), url(cid:logo)"),
            vec!["http://c.com/x.png"]
        );
    }

    #[test]
    fn safe_srcset_descriptor_forms() {
        assert!(safe_srcset_descriptor("2x"));
        assert!(safe_srcset_descriptor("1.5x"));
        assert!(safe_srcset_descriptor("640w"));
        assert!(!safe_srcset_descriptor("javascript"));
    }

    #[test]
    fn attribute_map_and_dimensions() {
        let m = get_attribute_map(r#"src="x.png" WIDTH='10' data-y=z"#);
        assert_eq!(m.get("src").map(String::as_str), Some("x.png"));
        assert_eq!(m.get("width").map(String::as_str), Some("10"));
        assert_eq!(m.get("data-y").map(String::as_str), Some("z"));
        assert_eq!(parse_pixel_dimension(Some("10px")), Some(10.0));
        assert_eq!(parse_pixel_dimension(Some("12")), Some(12.0));
        assert_eq!(parse_pixel_dimension(Some("auto")), None);
        assert_eq!(parse_pixel_dimension(None), None);
    }

    #[test]
    fn inline_style_property_and_text_render() {
        assert_eq!(get_inline_style_property("width: 10px; color: red", "width").as_deref(), Some("10px"));
        assert_eq!(get_inline_style_property("color:red", "width"), None);
        assert_eq!(render_text_preview("a\nb"), "<div>a<br>b</div>");
    }

    #[test]
    fn extract_remote_urls_ordered_dedups_in_order() {
        let urls = extract_remote_urls_ordered(
            r#"<img src="http://a.com/1"><img src="http://b.com/2"><img src="http://a.com/1">"#,
        );
        assert_eq!(urls, vec!["http://a.com/1", "http://b.com/2"]);
    }

    #[test]
    fn apple_inline_photo_renders_in_place_between_text_parts() {
        let eml = concat!(
            "From: a@example.com\r\n",
            "Subject: photo\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=BOUND\r\n",
            "\r\n",
            "--BOUND\r\n",
            "Content-Type: text/plain; charset=us-ascii\r\n",
            "\r\n",
            "Look at this!\r\n",
            "--BOUND\r\n",
            "Content-Type: image/jpeg; name=IMG_1.JPG\r\n",
            "Content-Disposition: inline; filename=IMG_1.JPG\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "/9j/fakejpegbytes\r\n",
            "--BOUND\r\n",
            "Content-Type: text/plain; charset=us-ascii\r\n",
            "\r\n",
            "Sent from my iPhone\r\n",
            "--BOUND--\r\n",
        );
        let msg = mail_parser::MessageParser::default()
            .parse(eml.as_bytes())
            .unwrap();
        let html = build_preview_body(&msg, &|part: &mail_parser::MessagePart| Some(part.contents().to_vec()));
        let text1 = html.find("Look at this!").expect("first text part rendered");
        let img = html
            .find("data:image/jpeg;base64,")
            .expect("inline photo rendered as data URI");
        let text2 = html
            .find("Sent from my iPhone")
            .expect("second text part rendered");
        assert!(text1 < img && img < text2, "parts in original MIME order");
    }

    #[test]
    fn cid_referenced_inline_images_are_not_duplicated() {
        let eml = concat!(
            "From: a@example.com\r\n",
            "Subject: logo\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/related; boundary=BOUND\r\n",
            "\r\n",
            "--BOUND\r\n",
            "Content-Type: text/html; charset=us-ascii\r\n",
            "\r\n",
            "<p>hi</p><img src=\"cid:logo1\">\r\n",
            "--BOUND\r\n",
            "Content-Type: image/png\r\n",
            "Content-ID: <logo1>\r\n",
            "Content-Disposition: inline\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "iVBORw0KGgofake\r\n",
            "--BOUND--\r\n",
        );
        let msg = mail_parser::MessageParser::default()
            .parse(eml.as_bytes())
            .unwrap();
        let html = build_preview_body(&msg, &|part: &mail_parser::MessagePart| Some(part.contents().to_vec()));
        assert_eq!(
            html.matches("data:image/png;base64,").count(),
            1,
            "cid image appears once (in the body), not appended again"
        );
    }
}

#[cfg(test)]
mod hollow_tests {
    use super::*;

    /// A hollowed inline photo (empty body + marker) must render in place
    /// with bytes resolved from the blob store.
    #[test]
    fn hollowed_inline_photo_resolves_from_blob_store() {
        let eml = concat!(
            "From: a@example.com\r\n",
            "Subject: photo\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=BOUND\r\n",
            "\r\n",
            "--BOUND\r\n",
            "Content-Type: text/plain; charset=us-ascii\r\n",
            "\r\n",
            "Look at this!\r\n",
            "--BOUND\r\n",
            "Content-Type: image/jpeg; name=IMG_1.JPG\r\n",
            "Content-Disposition: inline; filename=IMG_1.JPG\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "X-PEA-Attachment: abc123hash\r\n",
            "\r\n",
            "--BOUND\r\n",
            "Content-Type: text/plain; charset=us-ascii\r\n",
            "\r\n",
            "Sent from my iPhone\r\n",
            "--BOUND--\r\n",
        );
        let msg = mail_parser::MessageParser::default()
            .parse(eml.as_bytes())
            .unwrap();
        let resolve = |part: &mail_parser::MessagePart| -> Option<Vec<u8>> {
            let contents = part.contents();
            if !contents.is_empty() {
                return Some(contents.to_vec());
            }
            (crate::ingest::part_pea_marker(part)? == "abc123hash")
                .then(|| b"fake jpeg bytes".to_vec())
        };
        let html = build_preview_body(&msg, &resolve);
        let text1 = html.find("Look at this!").expect("first text part");
        let img = html
            .find("data:image/jpeg;base64,")
            .expect("hollowed photo resolved and rendered");
        let text2 = html.find("Sent from my iPhone").expect("second text part");
        assert!(text1 < img && img < text2, "original MIME order");
    }
}
