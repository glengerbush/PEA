//! Port of the Node ingestion pipeline: IngestionService.processEmail
//! (dedupe, fingerprints, attachment stripping + dedup, blob storage,
//! DB rows) and IndexingService/SearchService (FTS document build + insert).
//! Readers live in readers.rs / eml.rs; job orchestration in
//! processors.rs. `import_mbox` drives the same queue pipeline from the CLI.
//!
//! Known divergences from Node (verified by scripts/golden-import.mjs):
//!  - stored .eml bytes for emails with attachments: Node re-composed the
//!    message; this engine "hollows" it instead (original bytes with each
//!    attachment body removed and an X-PEA-Attachment marker added; the
//!    /eml endpoint splices the blobs back for download)
//!  - extracted text for pdf/docx/xlsx attachments (different extractors)

use crate::state::AppState;
use mail_parser::{Message, MessageParser, MimeHeaders};
use std::sync::LazyLock;
use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

fn sha256_hex(data: &[u8]) -> String {
    crate::hex_encode(Sha256::digest(data))
}

fn now_ms() -> i64 {
    // Fall back to the epoch rather than panic if the system clock is set before
    // 1970 — a bogus timestamp is recoverable, an aborted import is not.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Fingerprint helpers (IngestionService head)
// ---------------------------------------------------------------------------

fn normalize_source_path(path: &str) -> String {
    path.split(['\\', '/'])
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_duplicate_text(value: &str) -> String {
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
    static NON_ALNUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").unwrap());
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let v = TAGS.replace_all(value, " ").to_lowercase();
    let v = NON_ALNUM.replace_all(&v, " ");
    WS.replace_all(v.trim(), " ").into_owned()
}

fn duplicate_hash(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(sha256_hex(value.as_bytes()))
    }
}

fn sanitize_filename(name: &str) -> String {
    static SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\\/ ]+").unwrap());
    let base = name.trim().rsplit('/').next().unwrap_or("").to_string();
    let cleaned = SEP.replace_all(&base, "_").trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "file".into()
    } else {
        cleaned
    }
}

/// Makes a value safe to use as a single path segment. The Message-ID (which
/// becomes the stored `.eml` filename) is copied verbatim from untrusted email
/// headers and can legally contain `/`, so strip separators and control chars
/// and neutralise `.`/`..` before it reaches the filesystem.
fn sanitize_path_component(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

/// IndexingService.sanitizeText — strips U+FFFD and control chars, trims.
pub(crate) fn sanitize_text(text: &str) -> String {
    static CTRL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new("[\u{0000}-\u{0008}\u{000B}\u{000C}\u{000E}-\u{001F}\u{007F}]").unwrap());
    let v = text.replace('\u{FFFD}', "");
    CTRL.replace_all(&v, "").trim().to_string()
}

// ---------------------------------------------------------------------------
// Parsed email (EmailObject equivalent)
// ---------------------------------------------------------------------------

pub struct EmailAddr {
    pub name: String,
    pub address: String,
}

pub struct AttachmentObj {
    pub filename: String,
    pub content_type: Option<String>,
    pub content: Vec<u8>,
    /// Content-Description header, when the sender included one.
    pub content_description: Option<String>,
    /// creation-date / modification-date Content-Disposition parameters
    /// (RFC 2183 file timestamps, kept as the sender's original strings).
    pub creation_date: Option<String>,
    pub modification_date: Option<String>,
}

pub struct EmailObj {
    /// message-id (or generated-<sha>) — becomes providerMessageId & the storage filename.
    pub id: String,
    /// The literal Message-ID header (with brackets) when the email had one.
    pub header_message_id: Option<String>,
    pub thread_id: Option<String>,
    pub from: Vec<EmailAddr>,
    pub to: Vec<EmailAddr>,
    pub cc: Vec<EmailAddr>,
    pub bcc: Vec<EmailAddr>,
    pub subject: String,
    pub body: String,
    pub html: String,
    pub attachments: Vec<AttachmentObj>,
    pub received_at_ms: i64,
    pub path: String,
    /// All folder/label tags for this message (e.g. every X-Gmail-Label). The
    /// first is also `path`; the rest would otherwise be silently discarded.
    pub labels: Vec<String>,
    pub raw: Vec<u8>,
}

/// Raw header value, trimmed. For the headers used here (message-id,
/// references, x-folder, x-gmail-labels) this matches mailparser's strings.
fn header_string(msg: &Message, name: &str) -> Option<String> {
    let value = msg.header_raw(name)?;
    Some(value.trim().to_string())
}

/// getThreadId — references[0] → in-reply-to → conversation-id → message-id.
fn thread_id_for(msg: &Message) -> Option<String> {
    if let Some(refs) = header_string(msg, "references") {
        // split on any RFC5322 whitespace (incl. CRLF/tab folds), so a folded
        // References header still yields just the root message-id.
        if let Some(first) = refs.split_whitespace().next() {
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
    }
    if let Some(irt) = header_string(msg, "in-reply-to") {
        if !irt.trim().is_empty() {
            return Some(irt.trim().to_string());
        }
    }
    if let Some(cid) = header_string(msg, "conversation-id") {
        if !cid.trim().is_empty() {
            return Some(cid.trim().to_string());
        }
    }
    if let Some(mid) = header_string(msg, "message-id") {
        if !mid.trim().is_empty() {
            return Some(mid.trim().to_string());
        }
    }
    None
}

fn map_addresses(addr: Option<&mail_parser::Address>) -> Vec<EmailAddr> {
    let Some(addr) = addr else { return Vec::new() };
    addr.iter()
        .map(|a| EmailAddr {
            name: a.name().unwrap_or("").to_string(),
            address: a.address().unwrap_or("").replace('\'', ""),
        })
        .collect()
}

pub(crate) fn part_content_type(part: &mail_parser::MessagePart) -> Option<String> {
    part.content_type().map(|ct| match ct.subtype() {
        Some(sub) => format!("{}/{}", ct.ctype(), sub).to_lowercase(),
        None => ct.ctype().to_lowercase(),
    })
}

/// Minimal port of html-to-text v9 defaults (as mailparser calls it):
/// paragraphs/blocks → blank lines, <br> → newline, headings uppercased,
/// `<a href>` → "text [href]" (skipped for #anchors), `<img>` → "alt [src]".
/// Whitespace details differ (no 80-col wrapping); consumers only need the
/// token sequence (duplicate hashes normalize, FTS comparisons collapse ws).
fn html_to_text(html: &str) -> String {
    static SCRIPT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<script\b[^>]*>.*?</script\s*>").unwrap());
    static STYLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<style\b[^>]*>.*?</style\s*>").unwrap());
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<head\b[^>]*>.*?</head\s*>").unwrap());
    static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(/?)([a-z][a-z0-9]*)([^>]*)>").unwrap());
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\r\n]+").unwrap());
    static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());
    static BLOCKS: [&str; 12] = [
        "p", "div", "table", "tr", "ul", "ol", "li", "blockquote", "pre", "section", "article",
        "header",
    ];

    let s = SCRIPT.replace_all(html, "");
    let s = STYLE.replace_all(&s, "");
    let s = HEAD.replace_all(&s, "");
    let s = COMMENT.replace_all(&s, "");

    let attr_of = |attrs: &str, name: &str| -> Option<String> {
        let re = Regex::new(&format!(
            r#"(?i)\b{name}\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))"#
        ))
        .ok()?;
        let m = re.captures(attrs)?;
        m.get(2)
            .or_else(|| m.get(3))
            .or_else(|| m.get(4))
            .map(|g| crate::preview::decode_html_attribute(g.as_str()))
    };

    let mut out = String::new();
    let mut cursor = 0usize;
    let mut a_href: Option<String> = None;
    let mut heading_depth = 0u32;
    let push_text = |out: &mut String, text: &str, heading: bool| {
        let decoded = crate::preview::decode_html_attribute(text);
        let collapsed = WS.replace_all(&decoded, " ");
        if collapsed.trim().is_empty() {
            return;
        }
        let value = if heading {
            collapsed.to_uppercase()
        } else {
            collapsed.into_owned()
        };
        out.push_str(&value);
    };
    for m in TAG.captures_iter(&s) {
        let whole = m.get(0).unwrap();
        push_text(&mut out, &s[cursor..whole.start()], heading_depth > 0);
        cursor = whole.end();
        let closing = !m[1].is_empty();
        let name = m[2].to_lowercase();
        let attrs = m.get(3).map(|g| g.as_str()).unwrap_or("");
        let is_heading = name.len() == 2 && name.starts_with('h') && name[1..].chars().all(|c| c.is_ascii_digit());
        if closing {
            if BLOCKS.contains(&name.as_str()) {
                out.push_str("\n\n");
            } else if is_heading {
                heading_depth = heading_depth.saturating_sub(1);
                out.push_str("\n\n");
            } else if name == "a" {
                if let Some(href) = a_href.take() {
                    let href = href.trim();
                    if !href.is_empty() && !href.starts_with('#') {
                        out.push_str(&format!(" [{href}]"));
                    }
                }
            }
        } else {
            match name.as_str() {
                "br" => out.push('\n'),
                "img" => {
                    let alt = attr_of(attrs, "alt").unwrap_or_default();
                    let src = attr_of(attrs, "src").unwrap_or_default();
                    match (alt.is_empty(), src.is_empty()) {
                        (false, false) => out.push_str(&format!("{alt} [{src}]")),
                        (false, true) => out.push_str(&alt),
                        (true, false) => out.push_str(&format!("[{src}]")),
                        (true, true) => {}
                    }
                }
                "a" => a_href = attr_of(attrs, "href"),
                _ if BLOCKS.contains(&name.as_str()) => out.push_str("\n\n"),
                _ if is_heading => {
                    heading_depth += 1;
                    out.push_str("\n\n");
                }
                _ => {}
            }
        }
    }
    push_text(&mut out, &s[cursor..], heading_depth > 0);

    // Tidy: strip spaces around newlines, cap blank runs, trim.
    static SPACE_NL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" *\n *").unwrap());
    let out = SPACE_NL.replace_all(&out, "\n");
    BLANKS.replace_all(&out, "\n\n").trim().to_string()
}

/// mailparser html/text semantics:
///  - html: html parts joined with `<br/>\n`, with `cid:` image references
///    replaced by data: URIs for matching image/* attachments.
///  - text: text parts joined with `\n`; when the message is a bare text/html
///    root with no text part, htmlToText(html); otherwise empty.
pub(crate) fn mailparser_text_and_html(msg: &Message) -> (String, String) {
    // mail-parser auto-converts (text_body points at the html part for
    // html-only mail and vice versa) — filter by the part's REAL type so the
    // semantics match mailparser exactly.
    let html_parts: Vec<String> = msg
        .html_body
        .iter()
        .filter_map(|&id| {
            let part = msg.part(id)?;
            if part_content_type(part).as_deref() == Some("text/html") {
                part.text_contents().map(String::from)
            } else {
                None
            }
        })
        .collect();
    let text_parts: Vec<String> = msg
        .text_body
        .iter()
        .filter_map(|&id| {
            let part = msg.part(id)?;
            if part_content_type(part).as_deref() == Some("text/html") {
                None
            } else {
                part.text_contents().map(String::from)
            }
        })
        .collect();

    let mut html = html_parts.join("<br/>\n");
    if !html.is_empty() {
        static CID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bcid:([^'"\s]{1,256})"#).unwrap());
        static IMAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^image/\w+$").unwrap());
        html = CID
            .replace_all(&html, |c: &regex::Captures| {
                let cid = &c[1];
                for part in msg.attachments() {
                    let part_cid = part
                        .content_id()
                        .map(|p| p.trim_start_matches('<').trim_end_matches('>'));
                    if part_cid == Some(cid) {
                        // Hollowed parts have no bytes here; leave the cid:
                        // reference for the preview to resolve from storage.
                        if part.contents().is_empty() {
                            continue;
                        }
                        if let Some(ct) = part_content_type(part) {
                            if IMAGE.is_match(&ct) {
                                use base64::Engine as _;
                                return format!(
                                    "data:{};base64,{}",
                                    ct,
                                    base64::engine::general_purpose::STANDARD
                                        .encode(part.contents())
                                );
                            }
                        }
                    }
                }
                c[0].to_string()
            })
            .into_owned();
    }

    // Bare text/html root (mailparser: node.root && !hasText → htmlToText).
    let root_is_html = msg
        .part(0)
        .map_or(false, |p| part_content_type(p).as_deref() == Some("text/html"));
    let text = if !text_parts.is_empty() {
        text_parts.join("\n")
    } else if !html_parts.is_empty() && root_is_html {
        html_to_text(&html)
    } else {
        String::new()
    };
    (text, html)
}

/// Mbox reader: parse — a clean RFC 5322 buffer to an EmailObj.
/// Metadata Apple Mail keeps in the emlx plist trailer (alongside, not inside,
/// the RFC-822 message). Used as a fallback when the message itself is missing
/// headers — some Apple Mail messages are stored headerless/corrupt, but Apple
/// still recorded the date/subject/sender/to separately.
#[derive(Default, Debug, Clone)]
pub(crate) struct EmlxMeta {
    pub date_sent_ms: Option<i64>,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub to: Option<String>,
}

/// Parse a display-form address string ("Name <addr>" or a bare address) into an
/// EmailAddr, for the emlx-plist sender/to fallback. Returns None if there's no
/// usable address.
fn parse_display_address(raw: &str) -> Option<EmailAddr> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let (Some(lt), Some(gt)) = (raw.rfind('<'), raw.rfind('>')) {
        if lt < gt {
            let address = raw[lt + 1..gt].trim().to_string();
            let name = raw[..lt].trim().trim_matches('"').trim().to_string();
            if !address.is_empty() {
                let name = if name.is_empty() { address.clone() } else { name };
                return Some(EmailAddr { name, address });
            }
        }
    }
    if raw.contains('@') {
        return Some(EmailAddr { name: raw.to_string(), address: raw.to_string() });
    }
    None
}

pub(crate) fn parse_message(
    eml: Vec<u8>,
    source_path: &str,
    meta: Option<&EmlxMeta>,
) -> Result<EmailObj, String> {
    let msg = MessageParser::default()
        .parse(&eml)
        .ok_or("unparseable message")?;

    let (text, html) = mailparser_text_and_html(&msg);
    let has_content = msg.headers().len() > 0 || !text.trim().is_empty() || !html.trim().is_empty();
    if !has_content {
        return Err("Mbox message did not contain headers or body content.".into());
    }

    let attachments: Vec<AttachmentObj> = msg
        .attachments()
        .map(|part| {
            let disposition = part.content_disposition();
            AttachmentObj {
                filename: part
                    .attachment_name()
                    .map(String::from)
                    .unwrap_or_else(|| "untitled".into()),
                content_type: part_content_type(part),
                content: part.contents().to_vec(),
                content_description: part.content_description().map(String::from),
                creation_date: disposition
                    .and_then(|d| d.attribute("creation-date"))
                    .map(String::from),
                modification_date: disposition
                    .and_then(|d| d.attribute("modification-date"))
                    .map(String::from),
            }
        })
        .collect();

    let thread_id = thread_id_for(&msg);
    // mailparser keeps the <> brackets on messageId.
    let header_message_id = msg.message_id().map(|mid| format!("<{}>", mid));
    let message_id = header_message_id
        .clone()
        .unwrap_or_else(|| format!("generated-{}", sha256_hex(&eml)));

    // When the message has no usable From (e.g. a corrupt/headerless Apple Mail
    // message), fall back to the emlx plist's sender before the placeholder.
    let mut from = map_addresses(msg.from());
    if from.is_empty() {
        from = meta
            .and_then(|m| m.sender.as_deref())
            .and_then(parse_display_address)
            .map(|addr| vec![addr])
            .unwrap_or_else(|| {
                vec![EmailAddr {
                    name: "No Sender".into(),
                    address: "No Sender".into(),
                }]
            });
    }

    // Folder from client headers: X-Gmail-Labels → X-Folder → source path.
    // Gmail Takeout stores each message once with ALL its labels in one header,
    // so the folder-tag-merge path (which only fires on re-ingest) never sees
    // the others. Keep the first label as the folder/path but carry every label
    // as a tag, or the message is findable only under its first label.
    let mut final_path = source_path.to_string();
    let mut labels: Vec<String> = Vec::new();
    if let Some(raw_labels) = header_string(&msg, "x-gmail-labels") {
        labels = raw_labels
            .split(',')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if let Some(first) = labels.first() {
            final_path = first.clone();
        }
    } else if let Some(folder) = header_string(&msg, "x-folder") {
        final_path = folder;
    }

    // Prefer the message's Date header; fall back to the emlx plist's date-sent
    // (Apple keeps it even when the body has no Date), and only then to now().
    let received_at_ms = msg
        .date()
        // mail_parser returns Some for a malformed-but-parseable Date (e.g.
        // "47:99:99" or an out-of-range year) without validating it; skip those
        // so we fall back to the emlx date / now() instead of a bogus sent time.
        .filter(|d| d.is_valid())
        .map(|d| d.to_timestamp() * 1000)
        .or_else(|| meta.and_then(|m| m.date_sent_ms))
        .unwrap_or_else(now_ms);

    // Recipients / subject: same plist fallback when the message omits them.
    let mut to = map_addresses(msg.to());
    if to.is_empty() {
        if let Some(addr) = meta.and_then(|m| m.to.as_deref()).and_then(parse_display_address) {
            to.push(addr);
        }
    }
    let subject = {
        let s = msg.subject().unwrap_or("").trim().to_string();
        if s.is_empty() {
            meta.and_then(|m| m.subject.clone()).unwrap_or_default()
        } else {
            s
        }
    };

    Ok(EmailObj {
        id: message_id,
        header_message_id,
        thread_id,
        to,
        cc: map_addresses(msg.cc()),
        bcc: map_addresses(msg.bcc()),
        from,
        subject,
        body: text,
        html,
        attachments,
        received_at_ms,
        path: final_path,
        labels,
        raw: eml,
    })
}

// ---------------------------------------------------------------------------
// Mbox splitting (MboxSplitter + envelope/quoting cleanup)
// ---------------------------------------------------------------------------

// (mbox splitting/unescaping lives in readers.rs now)

// ---------------------------------------------------------------------------
// Attachment stripping (emlUtils.stripAttachmentsFromEml)
// ---------------------------------------------------------------------------

pub(crate) fn extract_cid_references(html: &str) -> std::collections::HashSet<String> {
    // Exclude `)` too, so an unquoted CSS `url(cid:hero)` captures `hero`, not
    // `hero)` — otherwise the part reads as unreferenced and renders twice.
    static CID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\bcid:([^\s"'>)]+)"#).unwrap());
    CID.captures_iter(html)
        // Normalize identically to the cid_map keys: strip angle brackets and
        // lowercase, so `cid:<logo>` matches a `Content-ID: <logo>` part.
        .map(|m| m[1].trim_start_matches('<').trim_end_matches('>').to_lowercase())
        .collect()
}

/// Marker header added to hollowed attachment parts; its value is the
/// sha-256 of the decoded part contents, which is also the blob-store key.
pub(crate) const PEA_ATTACHMENT_MARKER: &str = "X-PEA-Attachment";

/// The sha-256 blob key of a hollowed part, when this part was hollowed.
/// Scans headers in REVERSE so a part that already carried a (foreign)
/// X-PEA-Attachment header can't shadow the one hollowing appended last.
pub(crate) fn part_pea_marker(part: &mail_parser::MessagePart) -> Option<String> {
    part.headers.iter().rev().find_map(|header| {
        if !header.name.as_str().eq_ignore_ascii_case(PEA_ATTACHMENT_MARKER) {
            return None;
        }
        header.value.as_text().map(|v| v.trim().to_string())
    })
}

/// Line-ending style of the header/body separator ending at `offset_body`.
fn body_separator_eol(eml: &[u8], offset_body: usize) -> &'static [u8] {
    if offset_body >= 2 && &eml[offset_body - 2..offset_body] == b"\r\n" {
        b"\r\n"
    } else {
        b"\n"
    }
}

/// "Hollows" every attachment part: the original bytes are kept verbatim —
/// headers, folding, boundaries, part order — except each attachment body is
/// removed and an X-PEA-Attachment marker (the blob-store sha-256) is added
/// to that part's headers. The bytes live once, decoded, in the attachment
/// store; `rebuild_eml` splices them back for download.
fn hollow_attachments_from_eml(eml: &[u8]) -> Vec<u8> {
    let Some(msg) = MessageParser::default().parse(eml) else {
        return eml.to_vec();
    };
    if msg.attachment_count() == 0 {
        return eml.to_vec();
    }

    // (offset_header, offset_body, offset_end, content sha) per hollowable part.
    let mut regions: Vec<(usize, usize, usize, String)> = msg
        .attachments()
        .filter_map(|part| {
            let (header, body, end) = (part.offset_header, part.offset_body, part.offset_end);
            if body == 0 || body > end || end > eml.len() || part.contents().is_empty() {
                return None;
            }
            Some((header, body, end, sha256_hex(part.contents())))
        })
        .collect();
    if regions.is_empty() {
        return eml.to_vec();
    }
    regions.sort_by_key(|r| r.0);

    let mut out = Vec::with_capacity(eml.len() / 2);
    let mut cursor = 0usize;
    for (offset_header, offset_body, offset_end, hash) in regions {
        // Skip parts nested inside an already-hollowed region (rfc822 parts).
        if offset_header < cursor {
            continue;
        }
        let eol = body_separator_eol(eml, offset_body);
        let blank_start = offset_body.saturating_sub(eol.len());
        // Original part headers, then the marker, then the blank line. The
        // body span excludes the CRLF preceding the next boundary, so an
        // empty body needs nothing further.
        out.extend_from_slice(&eml[cursor..blank_start]);
        out.extend_from_slice(format!("{PEA_ATTACHMENT_MARKER}: {hash}").as_bytes());
        out.extend_from_slice(eol);
        out.extend_from_slice(eol);
        cursor = offset_end;
    }
    out.extend_from_slice(&eml[cursor..]);
    out
}

/// Wraps base64 output at the RFC 2045 76-character line length. No trailing
/// line break — the part body span excludes the CRLF before the boundary.
fn base64_wrapped(bytes: &[u8], eol: &[u8]) -> Vec<u8> {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut out = Vec::with_capacity(encoded.len() + encoded.len() / 76 * eol.len());
    for (i, chunk) in encoded.as_bytes().chunks(76).enumerate() {
        if i > 0 {
            out.extend_from_slice(eol);
        }
        out.extend_from_slice(chunk);
    }
    out
}

/// Reconstructs a downloadable .eml from a hollowed one: each marker part
/// gets its blob spliced back (re-encoded per the part's declared
/// Content-Transfer-Encoding) and the marker header removed. Emails stored
/// before hollowing existed have no markers and pass through unchanged.
pub(crate) fn rebuild_eml(stored: &[u8], load_blob: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<u8> {
    let Some(msg) = MessageParser::default().parse(stored) else {
        return stored.to_vec();
    };

    struct Splice {
        offset_header: usize,
        offset_body: usize,
        offset_end: usize,
        hash: String,
        encoding: Option<String>,
    }
    let mut splices: Vec<Splice> = msg
        .attachments()
        .filter_map(|part| {
            let hash = part_pea_marker(part)?;
            Some(Splice {
                offset_header: part.offset_header,
                offset_body: part.offset_body,
                offset_end: part.offset_end,
                hash,
                encoding: part
                    .content_transfer_encoding()
                    .map(|e| e.trim().to_lowercase()),
            })
        })
        .collect();
    if splices.is_empty() {
        return stored.to_vec();
    }
    splices.sort_by_key(|s| s.offset_header);

    let mut out = Vec::with_capacity(stored.len());
    let mut cursor = 0usize;
    for splice in splices {
        if splice.offset_header < cursor || splice.offset_end > stored.len() {
            continue;
        }
        let Some(bytes) = load_blob(&splice.hash) else {
            continue; // blob missing: leave the hollowed part as stored
        };
        let eol = body_separator_eol(stored, splice.offset_body);

        // mail-parser hands us DECODED bytes for both base64 and
        // quoted-printable, so both must be re-encoded on the way out. We
        // re-encode quoted-printable parts as base64 (universally decodable,
        // no fragile QP encoder) and rewrite that part's CTE header to match.
        let reencode_base64 = matches!(
            splice.encoding.as_deref(),
            Some("base64") | Some("quoted-printable")
        );
        let is_qp = splice.encoding.as_deref() == Some("quoted-printable");

        // Copy the part headers, dropping the marker line and — for a QP part
        // being rebased to base64 — rewriting its Content-Transfer-Encoding.
        let header_block = &stored[splice.offset_header..splice.offset_body];
        let marker_prefix = format!("{PEA_ATTACHMENT_MARKER}:").to_lowercase();
        out.extend_from_slice(&stored[cursor..splice.offset_header]);
        let mut line_start = 0usize;
        while line_start < header_block.len() {
            let line_end = header_block[line_start..]
                .iter()
                .position(|b| *b == b'\n')
                .map(|p| line_start + p + 1)
                .unwrap_or(header_block.len());
            let line = &header_block[line_start..line_end];
            let lower = String::from_utf8_lossy(line).to_lowercase();
            if lower.starts_with(&marker_prefix) {
                // drop the hollowing marker
            } else if is_qp && lower.starts_with("content-transfer-encoding:") {
                out.extend_from_slice(b"Content-Transfer-Encoding: base64");
                out.extend_from_slice(eol);
            } else {
                out.extend_from_slice(line);
            }
            line_start = line_end;
        }

        // Splice the body back, re-encoded to match the (possibly rewritten) CTE.
        if reencode_base64 {
            out.extend_from_slice(&base64_wrapped(&bytes, eol));
        } else {
            // 7bit/8bit/binary/none: the decoded bytes are the original body.
            out.extend_from_slice(&bytes);
        }
        cursor = splice.offset_end;
    }
    out.extend_from_slice(&stored[cursor..]);
    out
}

// ---------------------------------------------------------------------------
// Text extraction (helpers/textExtractor)
// ---------------------------------------------------------------------------

fn extract_text(buffer: &[u8], mime_type: &str) -> String {
    if buffer.is_empty() || mime_type.is_empty() || buffer.len() > 50 * 1024 * 1024 {
        return String::new();
    }
    if mime_type == "application/pdf" {
        // pdf_extract can PANIC (not just Err) on malformed/adversarial PDFs. A
        // panic mid-import would abort the whole batch, so isolate it and fail
        // closed (empty text) for this one attachment.
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pdf_extract::extract_text_from_mem(buffer).unwrap_or_default()
        }))
        .unwrap_or_default();
    }
    if mime_type == "application/vnd.openxmlformats-officedocument.wordprocessingml.document" {
        // calamine/quick_xml/zip can PANIC (not just Err) on a malformed Office
        // file — isolate it like PDF so one bad attachment can't abort the import.
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            extract_docx_text(buffer).unwrap_or_default()
        }))
        .unwrap_or_default();
    }
    if mime_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" {
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            extract_xlsx_text(buffer).unwrap_or_default()
        }))
        .unwrap_or_default();
    }
    if mime_type.starts_with("text/")
        || mime_type == "application/json"
        || mime_type == "application/xml"
    {
        return String::from_utf8_lossy(buffer).to_string();
    }
    String::new()
}

fn extract_docx_text(buffer: &[u8]) -> Option<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(buffer)).ok()?;
    let mut file = zip.by_name("word/document.xml").ok()?;
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut file, &mut xml).ok()?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) if e.local_name().as_ref() == b"t" => {
                in_text = true;
            }
            Ok(quick_xml::events::Event::End(e)) => {
                if e.local_name().as_ref() == b"t" {
                    in_text = false;
                } else if e.local_name().as_ref() == b"p" {
                    out.push_str("\n\n");
                }
            }
            Ok(quick_xml::events::Event::Text(t)) if in_text => {
                // quick-xml 0.41 split decode + unescape into separate steps.
                if let Ok(decoded) = t.decode() {
                    match quick_xml::escape::unescape(&decoded) {
                        Ok(s) => out.push_str(&s),
                        Err(_) => out.push_str(&decoded),
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    Some(out.trim().to_string())
}

fn extract_xlsx_text(buffer: &[u8]) -> Option<String> {
    use calamine::Reader;
    let mut workbook: calamine::Xlsx<_> =
        calamine::open_workbook_from_rs(std::io::Cursor::new(buffer)).ok()?;
    let mut full = String::new();
    for sheet in workbook.sheet_names() {
        if let Ok(range) = workbook.worksheet_range(&sheet) {
            for row in range.rows() {
                let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
                full.push_str(&cells.join("\t"));
                full.push('\n');
            }
        }
        full.push('\n');
    }
    Some(full.trim().to_string())
}

// ---------------------------------------------------------------------------
// The import pipeline
// ---------------------------------------------------------------------------

pub struct ImportStats {
    pub source_id: String,
    pub archived: usize,
    pub skipped_duplicates: usize,
    pub failed: usize,
}

/// CLI import — creates the source exactly like POST /ingestion-sources and
/// drains the job queue synchronously, so the CLI and the API share one code
/// path (initial-import → process-mailbox → index-email-batch → finished).
pub fn import_mbox(
    data_dir: &Path,
    mbox_path: &Path,
    name: Option<String>,
) -> Result<ImportStats, String> {
    let state = crate::state_for_dir(data_dir, false)?;
    let mbox_abs = std::fs::canonicalize(mbox_path).map_err(|e| e.to_string())?;
    let mut dto = json!({
        "provider": "mbox_import",
        "providerConfig": { "localFilePath": mbox_abs.to_string_lossy() },
    });
    if let Some(name) = name {
        dto["name"] = json!(name);
    }
    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let source_id = crate::sources::create_source(&state, &conn, &dto)?;
    drop(conn);
    crate::queue::drain_for_cli(&state)?;

    let conn = state.pool.get().map_err(|e| e.to_string())?;
    let archived: i64 = conn
        .query_row(
            "SELECT count(*) FROM archived_emails WHERE ingestion_source_id = ?",
            [&source_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(ImportStats {
        source_id,
        archived: archived as usize,
        skipped_duplicates: 0,
        failed: 0,
    })
}

/// IngestionService.processEmail — returns the archived email id, or None for
/// an intra-group duplicate (same message-id). `effective` is the merge-group
/// root that owns storage and DB rows; `group_ids` scope the dedupe check.
pub(crate) fn process_email(
    state: &AppState,
    conn: &Connection,
    source_id: &str,
    group_ids: &[String],
    effective: &crate::sources::SourceRow,
    email: &EmailObj,
    import_source: &str,
) -> Result<Option<String>, String> {
    // Node: header value if present, else generated-<sha(raw)>-<sourceId>-<email.id>.
    let message_id = &email.header_message_id.clone().unwrap_or_else(|| {
        format!("generated-{}-{}-{}", sha256_hex(&email.raw), source_id, email.id)
    });

    // Duplicate check within the merge group (standalone source → itself).
    let placeholders = vec!["?"; group_ids.len()].join(", ");
    let mut params: Vec<rusqlite::types::Value> = vec![message_id.clone().into()];
    params.extend(group_ids.iter().map(|s| rusqlite::types::Value::from(s.clone())));
    let existing: Option<String> = conn
        .query_row(
            &format!(
                "SELECT id FROM archived_emails WHERE message_id_header = ? \
                 AND ingestion_source_id IN ({placeholders})"
            ),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )
        .ok();
    if let Some(existing_id) = existing {
        // A duplicate isn't dropped silently: it still contributes its mbox
        // folder as a tag so the surviving email is findable under every
        // folder it appeared in.
        let folder_tag = normalize_source_path(&email.path);
        if !folder_tag.is_empty() {
            let current: Option<String> = conn
                .query_row(
                    "SELECT tags FROM archived_emails WHERE id = ?",
                    [&existing_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut tags: Vec<String> = current
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            if !tags.contains(&folder_tag) {
                tags.push(folder_tag);
                conn.execute(
                    "UPDATE archived_emails SET tags = ? WHERE id = ?",
                    rusqlite::params![serde_json::to_string(&tags).unwrap(), existing_id],
                )
                .map_err(|e| e.to_string())?;
                index_email(state, conn, &existing_id)?;
            }
        }
        return Ok(None);
    }

    let source_path = normalize_source_path(&email.path);
    let mut email_tags: Vec<String> = Vec::new();
    if !source_path.is_empty() {
        email_tags.push(source_path.clone());
    }
    // Every folder/label (e.g. all X-Gmail-Labels) becomes a tag so the message
    // is findable under each — not just the first, which is the source_path.
    for label in &email.labels {
        let tag = normalize_source_path(label);
        if !tag.is_empty() && !email_tags.contains(&tag) {
            email_tags.push(tag);
        }
    }
    let sender_email = email.from.first().map(|a| a.address.clone()).unwrap_or_default();
    let duplicate_subject_hash = duplicate_hash(&normalize_duplicate_text(&email.subject));
    let body_or_html = if !email.body.is_empty() { &email.body } else { &email.html };
    let duplicate_body_hash = duplicate_hash(&normalize_duplicate_text(body_or_html));
    let mut recipient_addresses: Vec<String> = email
        .to
        .iter()
        .chain(email.cc.iter())
        .chain(email.bcc.iter())
        .map(|a| a.address.trim().to_lowercase())
        .filter(|a| !a.is_empty())
        .collect();
    recipient_addresses.sort();
    let duplicate_recipient_fingerprint = if recipient_addresses.is_empty() {
        None
    } else {
        duplicate_hash(&recipient_addresses.join("|"))
    };

    let storage_path_segment = if source_path.is_empty() {
        String::new()
    } else {
        format!("{source_path}/")
    };
    // Storage and DB ownership always go to the merge-group root. email.id is
    // the untrusted Message-ID (see sanitize_path_component). effective.id only
    // disambiguates the SOURCE directory, not two emails within it, so a 7-char
    // uuid prefix on the filename keeps it unique even when two distinct
    // Message-IDs sanitise to the same string (e.g. `<a/b@h>` and `<a_b@h>`) —
    // otherwise the second email's .eml would silently overwrite the first's.
    let email_uniq = &uuid()[..7];
    let email_path = format!(
        "pea/{}-{}/emails/{}{}-{}.eml",
        effective.name.replace(' ', "-"),
        effective.id,
        storage_path_segment,
        email_uniq,
        sanitize_path_component(&email.id)
    );

    let eml_buffer = hollow_attachments_from_eml(&email.raw);
    let email_hash = sha256_hex(&eml_buffer);
    state.storage_put(&email_path, &eml_buffer)?;

    let addr_json = |list: &[EmailAddr]| -> Value {
        Value::Array(
            list.iter()
                .map(|a| json!({ "name": a.name, "address": a.address }))
                .collect(),
        )
    };
    let recipients = json!({
        "to": addr_json(&email.to),
        "cc": addr_json(&email.cc),
        "bcc": addr_json(&email.bcc),
    });

    let archived_id = uuid();
    // The row and its attachments are one unit. Without this transaction, a
    // failure while writing an attachment blob below (e.g. disk-full — the most
    // likely failure point on a large import) would leave a committed
    // has_attachments=true row whose blob was never written, and re-import's
    // message-id dedup would skip it forever → permanent silent attachment loss.
    // On any error the transaction rolls back, so re-import re-processes and heals.
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    conn
        .execute(
            "INSERT INTO archived_emails (id, ingestion_source_id, import_source, thread_id, \
             message_id_header, provider_message_id, sent_at, subject, sender_name, sender_email, \
             recipients, storage_path, storage_hash_sha256, size_bytes, has_attachments, \
             source_path, duplicate_subject_hash, \
             duplicate_body_hash, duplicate_recipient_fingerprint, duplicate_attachment_fingerprint, \
             tags) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                archived_id,
                effective.id,
                import_source,
                email.thread_id,
                message_id,
                email.id,
                email.received_at_ms,
                email.subject,
                email.from.first().map(|a| a.name.clone()),
                sender_email,
                recipients.to_string(),
                email_path,
                email_hash,
                eml_buffer.len() as i64,
                !email.attachments.is_empty(),
                source_path,
                duplicate_subject_hash,
                duplicate_body_hash,
                duplicate_recipient_fingerprint,
                Option::<String>::None,
                serde_json::to_string(&email_tags).unwrap(),
            ],
        )
        .map_err(|e| e.to_string())?;

    if !email.attachments.is_empty() {
        let mut attachment_hashes: Vec<String> = Vec::new();
        for attachment in &email.attachments {
            let attachment_hash = sha256_hex(&attachment.content);
            attachment_hashes.push(attachment_hash.clone());
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM attachments WHERE content_hash_sha256 = ? AND ingestion_source_id = ?",
                    rusqlite::params![attachment_hash, effective.id],
                    |r| r.get(0),
                )
                .ok();
            let attachment_id = match existing {
                Some(id) => id,
                None => {
                    let unique = &uuid()[..7];
                    let storage_path = format!(
                        "pea/{}-{}/attachments/{}-{}",
                        effective.name.replace(' ', "-"),
                        effective.id,
                        unique,
                        sanitize_filename(&attachment.filename)
                    );
                    state.storage_put(&storage_path, &attachment.content)?;
                    // Extract searchable text once here (parsing PDFs/xlsx is
                    // costly) and cache it so re-indexing never re-parses.
                    let extracted = extract_text(
                        &attachment.content,
                        attachment.content_type.as_deref().unwrap_or(""),
                    );
                    let id = uuid();
                    conn
                        .execute(
                            "INSERT INTO attachments (id, filename, mime_type, size_bytes, \
                             content_hash_sha256, storage_path, ingestion_source_id, \
                             content_description, original_created_at, original_modified_at, \
                             extracted_text) \
                             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                            rusqlite::params![
                                id,
                                attachment.filename,
                                attachment.content_type,
                                attachment.content.len() as i64,
                                attachment_hash,
                                storage_path,
                                effective.id,
                                attachment.content_description,
                                attachment.creation_date,
                                attachment.modification_date,
                                extracted,
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                    id
                }
            };
            conn
                .execute(
                    "INSERT INTO email_attachments (id, email_id, attachment_id) VALUES (?, ?, ?)",
                    rusqlite::params![uuid(), archived_id, attachment_id],
                )
                .map_err(|e| e.to_string())?;
        }
        attachment_hashes.sort();
        let fingerprint = duplicate_hash(&attachment_hashes.join("|"));
        conn
            .execute(
                "UPDATE archived_emails SET duplicate_attachment_fingerprint = ? WHERE id = ?",
                rusqlite::params![fingerprint, archived_id],
            )
            .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(archived_id))
}

/// IndexingService.indexEmailById + SearchService.addDocuments for one email.
pub(crate) fn index_email(state: &AppState, conn: &Connection, email_id: &str) -> Result<(), String> {
    struct Row {
        subject: Option<String>,
        sender_email: String,
        sender_name: Option<String>,
        recipients: Option<String>,
        storage_path: String,
        import_source: String,
        source_path: Option<String>,
        tags: Option<String>,
        has_attachments: bool,
    }
    let row = conn
        .query_row(
            "SELECT subject, sender_email, sender_name, recipients, storage_path, import_source, \
             source_path, tags, has_attachments \
             FROM archived_emails WHERE id = ?",
            [email_id],
            |r| {
                Ok(Row {
                    subject: r.get(0)?,
                    sender_email: r.get(1)?,
                    sender_name: r.get(2)?,
                    recipients: r.get(3)?,
                    storage_path: r.get(4)?,
                    import_source: r.get(5)?,
                    source_path: r.get(6)?,
                    tags: r.get(7)?,
                    has_attachments: r.get::<_, i64>(8)? != 0,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    // Body text: stored eml → text || html || whole file as text.
    let raw = state.storage_get(&row.storage_path)?;
    let body = match MessageParser::default().parse(&raw) {
        Some(msg) => {
            let (text, html) = mailparser_text_and_html(&msg);
            // A parseable email with no text/html part (e.g. attachments only)
            // has no indexable body — do NOT dump the raw MIME, or the hollowing
            // X-PEA-Attachment markers and boundaries leak into the FTS index.
            if !text.is_empty() {
                text
            } else {
                html
            }
        }
        // Not a parseable message: fall back to treating the file as text.
        None => extract_text(&raw, "text/plain"),
    };

    // Attachment text, from the cached extracted_text column (populated once at
    // ingest); only re-parse the blob when the cache is NULL (legacy rows).
    let mut attachment_texts: Vec<(String, String)> = Vec::new();
    if row.has_attachments {
        let mut stmt = conn
            .prepare(
                "SELECT a.filename, a.mime_type, a.storage_path, a.extracted_text \
                 FROM email_attachments ea \
                 INNER JOIN attachments a ON ea.attachment_id = a.id WHERE ea.email_id = ?",
            )
            .map_err(|e| e.to_string())?;
        let atts: Vec<(String, Option<String>, String, Option<String>)> = stmt
            .query_map([email_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        for (filename, mime, storage_path, cached) in atts {
            if let Some(text) = cached {
                attachment_texts.push((filename, text));
                continue;
            }
            if let Ok(content) = state.storage_get(&storage_path) {
                let text = extract_text(&content, mime.as_deref().unwrap_or(""));
                attachment_texts.push((filename, text));
            }
        }
    }

    // Assemble columns exactly as SearchService.addDocuments does, after
    // IndexingService's sanitizeObject pass (sanitizeText on every string).
    let recipients: Value = row
        .recipients
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!({}));
    let addresses = |key: &str| -> Vec<String> {
        recipients
            .get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| r.get("address").and_then(|a| a.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut all_recipients: Vec<String> = Vec::new();
    for key in ["to", "cc", "bcc"] {
        all_recipients.extend(addresses(key).into_iter().map(|a| sanitize_text(&a)));
    }
    let tags: Vec<String> = row
        .tags
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let subject = sanitize_text(row.subject.as_deref().unwrap_or(""));
    let body = sanitize_text(&body);
    let sender = format!(
        "{} {}",
        sanitize_text(&row.sender_email),
        sanitize_text(row.sender_name.as_deref().unwrap_or(""))
    )
    .trim()
    .to_string();
    let recipients_col = all_recipients.join(" ");
    let attachments_col = attachment_texts
        .iter()
        .map(|(f, c)| format!("{}\n{}", sanitize_text(f), sanitize_text(c)))
        .collect::<Vec<_>>()
        .join("\n");
    let source_path_value = row.source_path.clone().unwrap_or_default();
    let mut meta_parts: Vec<String> = vec![sanitize_text(&row.import_source), sanitize_text(&source_path_value)];
    meta_parts.extend(tags.iter().map(|t| sanitize_text(t)));
    let meta = meta_parts.join(" ");

    conn
        .execute(
            "DELETE FROM email_fts WHERE rowid = (SELECT rowid FROM archived_emails WHERE id = ?)",
            [email_id],
        )
        .ok();
    conn
        .execute(
            "INSERT INTO email_fts (rowid, email_id, subject, body, sender, recipients, attachments, meta) \
             SELECT rowid, ?, ?, ?, ?, ?, ?, ? FROM archived_emails WHERE id = ?",
            rusqlite::params![
                email_id,
                subject,
                body,
                sender,
                recipients_col,
                attachments_col,
                meta,
                email_id
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_source_path_cleans_segments() {
        assert_eq!(normalize_source_path("/a/./b/../c/"), "a/b/c");
        assert_eq!(normalize_source_path("A\\B"), "A/B");
        assert_eq!(normalize_source_path(""), "");
    }

    #[test]
    fn normalize_duplicate_text_strips_markup_and_case() {
        assert_eq!(normalize_duplicate_text("<b>Hello,  World!</b>"), "hello world");
        assert_eq!(normalize_duplicate_text("A---B"), "a b");
    }

    #[test]
    fn duplicate_hash_basics() {
        assert!(duplicate_hash("").is_none());
        assert_eq!(duplicate_hash("x"), duplicate_hash("x"));
        assert!(duplicate_hash("x") != duplicate_hash("y"));
    }

    #[test]
    fn sanitize_filename_and_path_component() {
        assert_eq!(sanitize_filename("../../evil name.sh"), "evil_name.sh");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_path_component("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_path_component(".."), "_");
        assert_eq!(sanitize_path_component("<id@host>"), "<id@host>");
    }

    #[test]
    fn sanitize_text_strips_control_and_replacement() {
        assert_eq!(sanitize_text("a\u{FFFD}b\u{0007}c "), "abc");
    }

    #[test]
    fn extract_cid_references_normalizes() {
        // case-folded, angle-brackets stripped, and deduped
        let refs = extract_cid_references(r#"<img src="cid:Logo"><img src='cid:<logo>'> url(cid:Hero)"#);
        assert!(refs.contains("logo"));
        assert!(refs.contains("hero"));
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn message_id_cannot_traverse_the_storage_path() {
        // A hostile Message-ID with path separators must collapse to a single
        // safe filename segment (no `..`, no `/`).
        let hostile = "<x/../../../../home/victim/.config/PWNED>";
        let safe = sanitize_path_component(hostile);
        assert!(!safe.contains('/'), "no separators survive: {safe}");
        assert!(!safe.contains('\\'));
        assert_ne!(safe, "..");
        // And the storage guard rejects a path built from a separator-bearing id.
        let root = std::path::Path::new("/data/storage");
        assert!(crate::state::resolve_within(root, "pea/n-id/emails/../../etc/x").is_err());
        assert!(crate::state::resolve_within(root, &format!("pea/n-id/emails/{safe}.eml")).is_ok());
    }

    /// Apple-Mail-style photo email: multipart/mixed with a text part, an
    /// inline (no CID) jpeg, and a trailing text part.
    fn apple_photo_eml() -> Vec<u8> {
        concat!(
            "From: a@example.com\r\n",
            "To: b@example.com\r\n",
            "Subject: photo\r\n",
            "Message-ID: <photo-1@example.com>\r\n",
            "Date: Wed, 3 Jun 2015 02:31:00 +0000\r\n",
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
            "ZmFrZSBqcGVnIGJ5dGVz\r\n",
            "--BOUND\r\n",
            "Content-Type: text/plain; charset=us-ascii\r\n",
            "\r\n",
            "Sent from my iPhone\r\n",
            "--BOUND--\r\n",
        )
        .as_bytes()
        .to_vec()
    }

    #[test]
    fn hollowing_removes_bodies_and_keeps_headers_verbatim() {
        let eml = apple_photo_eml();
        let stored = hollow_attachments_from_eml(&eml);

        let text = String::from_utf8(stored.clone()).unwrap();
        assert!(!text.contains("ZmFrZSBqcGVnIGJ5dGVz"), "jpeg bytes removed");
        assert!(text.contains("X-PEA-Attachment: "), "marker added");
        // Everything before the photo part is the original bytes, untouched.
        let original = String::from_utf8(eml.clone()).unwrap();
        let prefix = original.split("Content-Transfer-Encoding").next().unwrap();
        assert!(text.starts_with(prefix), "prefix bytes identical");

        let msg = MessageParser::default().parse(&stored).unwrap();
        let body = mailparser_text_and_html(&msg).0;
        assert!(body.contains("Look at this!"), "first text part kept");
        assert!(body.contains("Sent from my iPhone"), "second text part kept");
        let photo = msg
            .attachments()
            .find(|p| part_content_type(p).as_deref() == Some("image/jpeg"))
            .expect("photo part still present");
        assert!(photo.contents().is_empty(), "photo body hollowed");
        assert!(part_pea_marker(photo).is_some(), "marker readable");
        assert_eq!(
            photo.attachment_name(),
            Some("IMG_1.JPG"),
            "original part headers intact"
        );
    }

    #[test]
    fn rebuild_restores_original_bytes() {
        let eml = apple_photo_eml();
        let stored = hollow_attachments_from_eml(&eml);
        assert_ne!(stored, eml);

        // The blob store holds the decoded part contents, keyed by sha-256.
        let original = MessageParser::default().parse(&eml).unwrap();
        let photo = original.attachments().next().unwrap();
        let decoded = photo.contents().to_vec();
        let hash = sha256_hex(&decoded);

        let rebuilt = rebuild_eml(&stored, &|requested| {
            (requested == hash).then(|| decoded.clone())
        });
        assert_eq!(
            String::from_utf8(rebuilt).unwrap(),
            String::from_utf8(eml).unwrap(),
            "hollow + rebuild round-trips to the original bytes"
        );
    }

    #[test]
    fn rebuild_passes_unhollowed_emails_through() {
        let eml = apple_photo_eml();
        let rebuilt = rebuild_eml(&eml, &|_| None);
        assert_eq!(rebuilt, eml, "no markers: bytes unchanged");
    }

    #[test]
    fn rebuild_reencodes_quoted_printable_attachment() {
        let eml = concat!(
            "From: a@example.com\r\n",
            "Subject: qp\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=B\r\n",
            "\r\n",
            "--B\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "hello\r\n",
            "--B\r\n",
            "Content-Type: text/csv; name=d.csv\r\n",
            "Content-Disposition: attachment; filename=d.csv\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n",
            "\r\n",
            "a=3Db,c=3Dd\r\n",
            "--B--\r\n",
        )
        .as_bytes()
        .to_vec();

        // mail-parser stores the DECODED attachment body.
        let decoded = MessageParser::default()
            .parse(&eml)
            .unwrap()
            .attachments()
            .next()
            .unwrap()
            .contents()
            .to_vec();
        assert_eq!(decoded, b"a=b,c=d", "QP decoded content");
        let hash = sha256_hex(&decoded);

        let stored = hollow_attachments_from_eml(&eml);
        let rebuilt = rebuild_eml(&stored, &|h| (h == hash).then(|| decoded.clone()));
        let text = String::from_utf8_lossy(&rebuilt);
        assert!(text.contains("Content-Transfer-Encoding: base64"), "CTE rebased to base64");

        // The rebuilt attachment must decode back to the original bytes.
        let att = MessageParser::default().parse(&rebuilt).unwrap();
        assert_eq!(
            att.attachments().next().unwrap().contents(),
            b"a=b,c=d",
            "attachment decodes correctly after rebuild (not re-QP-mangled)"
        );
    }

    #[test]
    fn rebuild_ignores_a_foreign_pea_marker_header() {
        let eml = concat!(
            "From: a@example.com\r\nSubject: x\r\nMIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=B\r\n\r\n",
            "--B\r\nContent-Type: text/plain\r\n\r\nhi\r\n",
            "--B\r\n",
            "Content-Type: application/pdf; name=d.pdf\r\n",
            "Content-Disposition: attachment; filename=d.pdf\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "X-PEA-Attachment: deadbeefforeign\r\n",
            "\r\n",
            "JVBERi1mYWtlcGRm\r\n",
            "--B--\r\n",
        )
        .as_bytes()
        .to_vec();

        let decoded = MessageParser::default()
            .parse(&eml)
            .unwrap()
            .attachments()
            .next()
            .unwrap()
            .contents()
            .to_vec();
        let hash = sha256_hex(&decoded);

        let stored = hollow_attachments_from_eml(&eml);
        // Our appended marker (last) must win over the foreign one.
        let smsg = MessageParser::default().parse(&stored).unwrap();
        assert_eq!(
            part_pea_marker(smsg.attachments().next().unwrap()).as_deref(),
            Some(hash.as_str()),
            "reads our marker, not the foreign one"
        );

        let rebuilt = rebuild_eml(&stored, &|h| (h == hash).then(|| decoded.clone()));
        assert!(
            !String::from_utf8_lossy(&rebuilt).contains("deadbeefforeign"),
            "foreign marker header dropped on rebuild"
        );
        let att = MessageParser::default().parse(&rebuilt).unwrap();
        assert_eq!(
            att.attachments().next().unwrap().contents(),
            decoded.as_slice(),
            "correct blob spliced despite the foreign marker"
        );
    }
}

#[cfg(test)]
mod header_fold_tests {
    use super::*;

    /// Hollowing must never corrupt long pre-folded headers into blank
    /// lines — a blank line ends the header block and everything after it
    /// (the remaining headers!) renders as the message body.
    #[test]
    fn hollowed_headers_survive_long_folded_values() {
        let eml = concat!(
            "Received: from mail.example.com (mail.example.com [10.0.0.1])\r\n",
            "\tby mx.google.com with ESMTPS id abc123 (version=TLSv1 cipher=DHE-RSA-AES256-SHA bits=256/256)\r\n",
            "\tfor <me@example.com>; Tue,  2 Jun 2015 22:31:02 -0400 (EDT)\r\n",
            "Authentication-Results: mx.google.com; spf=pass (google.com: domain of a@b.com designates 10.0.0.1 as permitted sender)\r\n",
            "\tsmtp.mail=a@b.com; dkim=pass header.i=@b.com; dmarc=pass (p=REJECT dis=NONE) header.from=b.com\r\n",
            "From: a@example.com\r\n",
            "To: me@example.com\r\n",
            "Subject: folded headers\r\n",
            "Message-ID: <fold-1@example.com>\r\n",
            "Date: Tue, 2 Jun 2015 22:31:00 -0400\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=BOUND\r\n",
            "\r\n",
            "--BOUND\r\n",
            "Content-Type: text/plain; charset=us-ascii\r\n",
            "\r\n",
            "The actual body.\r\n",
            "--BOUND\r\n",
            "Content-Type: application/pdf; name=doc.pdf\r\n",
            "Content-Disposition: attachment; filename=doc.pdf\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "JVBERi1mYWtlcGRm\r\n",
            "--BOUND--\r\n",
        )
        .as_bytes()
        .to_vec();

        let stored = hollow_attachments_from_eml(&eml);
        assert_ne!(stored, eml, "pdf body must be hollowed");

        let text = String::from_utf8(stored.clone()).unwrap();
        let header_block = text.split("\r\n\r\n").next().unwrap();
        assert!(
            header_block.contains("Received"),
            "long headers stay inside the header block"
        );

        let msg = MessageParser::default().parse(&stored).unwrap();
        assert!(msg.header_raw("received").is_some(), "Received survives");
        assert!(
            msg.header_raw("authentication-results").is_some(),
            "Authentication-Results survives"
        );
        let body = mailparser_text_and_html(&msg).0;
        assert_eq!(body.trim(), "The actual body.");
        assert!(
            !body.contains("Received"),
            "no headers leak into the body: {body}"
        );
    }
}
