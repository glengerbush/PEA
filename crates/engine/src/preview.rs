//! Port of RemoteContentService.buildPreview + remote-asset endpoints.
//!
//! The CSS sanitization, URL rewriting, cid inlining, and tracking heuristics
//! are exact ports of the Node code. The final HTML pass uses ammonia with the
//! same policy sanitize-html enforced (allowed tags/attributes/schemes,
//! link hardening, img rewriting) — the *serialized markup* can differ in
//! insignificant ways (attribute order, entity escaping), so the golden-diff
//! harness compares the preview HTML semantically, not byte-for-byte.

use crate::crypto;
use crate::state::AppState;
use mail_parser::MimeHeaders;
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use base64::Engine as _;
use once_cell::sync::Lazy;
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

static TRACKING_URL_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
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

pub fn decode_html_attribute(value: &str) -> String {
    static AMP: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)&amp;").unwrap());
    static QUOT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)&quot;").unwrap());
    static APOS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)&#39;|&apos;").unwrap());
    static LT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)&lt;").unwrap());
    static GT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)&gt;").unwrap());
    static HEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)&#x([0-9a-f]+);").unwrap());
    static DEC: Lazy<Regex> = Lazy::new(|| Regex::new(r"&#(\d+);").unwrap());
    let v = AMP.replace_all(value, "&");
    let v = QUOT.replace_all(&v, "\"");
    let v = APOS.replace_all(&v, "'");
    let v = LT.replace_all(&v, "<");
    let v = GT.replace_all(&v, ">");
    let v = HEX.replace_all(&v, |c: &regex::Captures| {
        u32::from_str_radix(&c[1], 16)
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_default()
    });
    let v = DEC.replace_all(&v, |c: &regex::Captures| {
        c[1].parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_default()
    });
    v.into_owned()
}

pub fn to_remote_url(value: &str) -> Option<String> {
    static CTRL: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\x00-\x1f\x7f]+").unwrap());
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
    static URL_PAT: Lazy<Regex> = Lazy::new(|| {
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
static CSS_URL_REWRITE: Lazy<Regex> = Lazy::new(|| {
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
    static COMMENTS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)/\*.*?\*/").unwrap());
    static IMPORT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)@import\b[^;]*;?").unwrap());
    static EXPRESSION: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)expression\s*\([^)]*\)").unwrap());
    static BEHAVIOR: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)(?:behavior|-moz-binding)\s*:[^;}]*").unwrap());
    static JS_PROTO: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)javascript:").unwrap());
    let cleaned = COMMENTS.replace_all(css, "");
    let cleaned = IMPORT.replace_all(&cleaned, "");
    let cleaned = EXPRESSION.replace_all(&cleaned, "");
    let cleaned = BEHAVIOR.replace_all(&cleaned, "");
    let cleaned = JS_PROTO.replace_all(&cleaned, "");
    let cleaned = cleaned.replace(['<', '>'], "");
    rewrite_css_urls(&cleaned, rewrite_url).trim().to_string()
}

fn sanitize_style(value: &str, rewrite_url: &dyn Fn(&str) -> Option<String>) -> String {
    static IMPORT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)@import\b[^;]*;?").unwrap());
    static EXPRESSION: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)expression\s*\([^)]*\)").unwrap());
    static BEHAVIOR: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)(?:behavior|-moz-binding)\s*:[^;]*").unwrap());
    static JS_PROTO: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)javascript:").unwrap());
    let cleaned = IMPORT.replace_all(value, "");
    let cleaned = EXPRESSION.replace_all(&cleaned, "");
    let cleaned = BEHAVIOR.replace_all(&cleaned, "");
    let cleaned = JS_PROTO.replace_all(&cleaned, "");
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
    static CID: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^cid:(.+)$").unwrap());
    if let Some(m) = CID.captures(trimmed) {
        let cid = m[1].trim_start_matches('<').trim_end_matches('>').to_string();
        return cid_map.get(&cid).cloned();
    }
    static DATA_IMG: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)^data:image/(png|jpeg|gif|webp|avif);base64,").unwrap());
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
    static W: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+w$").unwrap());
    static X: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+(\.\d+)?x$").unwrap());
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
    static ATTR: Lazy<Regex> = Lazy::new(|| {
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
    static DIM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(\d+(?:\.\d+)?)(?:px)?$").unwrap());
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
    static HIDDEN: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?:^|;)\s*display\s*:\s*none").unwrap());
    static INVISIBLE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?:^|;)\s*visibility\s*:\s*hidden").unwrap());
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

fn is_likely_tracking_url(url: &str) -> bool {
    TRACKING_URL_PATTERNS.iter().any(|p| p.is_match(url))
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
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<([a-zA-Z][\w:-]*)([^>]*)>").unwrap());
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
    static NEWLINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\r?\n").unwrap());
    format!("<div>{}</div>", NEWLINE.replace_all(&escape_html(text), "<br>"))
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
        static LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<link\b[^>]*>").unwrap());
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
                let absolute = url::Url::parse(&href)
                    .ok()
                    .and_then(|base| base.join(raw).ok())
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
    static STYLE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?is)<style\b[^>]*>(.*?)</style\s*>").unwrap());
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

    // exclusiveFilter: drop <img> that lost both src and srcset.
    static EMPTY_IMG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<img\b[^>]*>").unwrap());
    static HAS_SRC: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\b(?:src|srcset)\s*="#).unwrap());
    let sanitized_body = EMPTY_IMG.replace_all(&sanitized_body, |c: &regex::Captures| {
        if HAS_SRC.is_match(&c[0]) {
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
    let file = app.storage_root().join(path);
    std::fs::read(&file)
        .ok()
        .and_then(|c| crypto::decrypt_storage(c, &app.storage_key).ok())
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

    // Same text/html semantics as mailparser (shared with the ingest pipeline);
    // parsedEmail.html already has cid: images replaced with data: URIs.
    let (text_part, html_part) = match &message {
        Some(msg) => crate::ingest::mailparser_text_and_html(msg),
        None => (String::new(), String::new()),
    };
    let html_source = html_part.clone();
    let html = if !html_part.trim().is_empty() {
        html_part.clone()
    } else {
        render_text_preview(&text_part)
    };

    // cid → data-URI map from safely-typed inline attachments (≤ 1 MiB).
    let mut cid_map: HashMap<String, String> = HashMap::new();
    if let Some(msg) = &message {
        for attachment in msg.attachments() {
            let Some(cid) = attachment.content_id() else { continue };
            let contents = attachment.contents();
            if contents.is_empty() || contents.len() > MAX_INLINE_CID_BYTES {
                continue;
            }
            let content_type = attachment.content_type().map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_string(),
            });
            let normalized = normalize_content_type(content_type.as_deref());
            if !is_safe_preview_content_type(normalized.as_deref()) {
                continue;
            }
            let cid = cid.trim_start_matches('<').trim_end_matches('>').to_string();
            cid_map.insert(
                cid,
                format!(
                    "data:{};base64,{}",
                    normalized.unwrap(),
                    base64::engine::general_purpose::STANDARD.encode(contents)
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
