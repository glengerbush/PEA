//! Port of the Node ingestion pipeline: IngestionService.processEmail
//! (dedupe, fingerprints, attachment stripping + dedup, encrypted storage,
//! DB rows) and IndexingService/SearchService (FTS document build + insert).
//! Connectors live in connectors.rs / eml.rs; job orchestration in
//! processors.rs. `import_mbox` drives the same queue pipeline from the CLI.
//!
//! Known divergences from Node (verified by scripts/golden-import.mjs):
//!  - re-composed .eml bytes for emails whose attachments were stripped
//!    (nodemailer vs mail-builder serialization; content is parse-equal)
//!  - extracted text for pdf/docx/xlsx attachments (different extractors)

use crate::state::AppState;
use mail_parser::{Message, MessageParser, MimeHeaders};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
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
    static TAGS: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").unwrap());
    static NON_ALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-z0-9]+").unwrap());
    static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
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

fn build_fuzzy_group_key(sender_email: &str, subject_hash: &Option<String>) -> Option<String> {
    let subject_hash = subject_hash.as_ref()?;
    if sender_email.is_empty() {
        return None;
    }
    duplicate_hash(&format!(
        "{}|{}",
        sender_email.trim().to_lowercase(),
        subject_hash
    ))
}

fn sanitize_filename(name: &str) -> String {
    static SEP: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\\/ ]+").unwrap());
    let base = name.trim().rsplit('/').next().unwrap_or("").to_string();
    let cleaned = SEP.replace_all(&base, "_").trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "file".into()
    } else {
        cleaned
    }
}

/// IndexingService.sanitizeText — strips U+FFFD and control chars, trims.
fn sanitize_text(text: &str) -> String {
    static CTRL: Lazy<Regex> =
        Lazy::new(|| Regex::new("[\u{0000}-\u{0008}\u{000B}\u{000C}\u{000E}-\u{001F}\u{007F}]").unwrap());
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
        if let Some(first) = refs.split(' ').next() {
            let t = first.trim();
            if !t.is_empty() {
                return Some(t.to_string());
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

fn part_content_type(part: &mail_parser::MessagePart) -> Option<String> {
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
    static SCRIPT: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?is)<script\b[^>]*>.*?</script\s*>").unwrap());
    static STYLE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?is)<style\b[^>]*>.*?</style\s*>").unwrap());
    static HEAD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<head\b[^>]*>.*?</head\s*>").unwrap());
    static COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(/?)([a-z][a-z0-9]*)([^>]*)>").unwrap());
    static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t\r\n]+").unwrap());
    static BLANKS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());
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
    static SPACE_NL: Lazy<Regex> = Lazy::new(|| Regex::new(r" *\n *").unwrap());
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
        static CID: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\bcid:([^'"\s]{1,256})"#).unwrap());
        static IMAGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^image/\w+$").unwrap());
        html = CID
            .replace_all(&html, |c: &regex::Captures| {
                let cid = &c[1];
                for part in msg.attachments() {
                    let part_cid = part
                        .content_id()
                        .map(|p| p.trim_start_matches('<').trim_end_matches('>'));
                    if part_cid == Some(cid) {
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

/// MboxConnector.parseMessage — a clean RFC 5322 buffer to an EmailObj.
pub(crate) fn parse_message(eml: Vec<u8>, source_path: &str) -> Result<EmailObj, String> {
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
        .map(|part| AttachmentObj {
            filename: part
                .attachment_name()
                .map(String::from)
                .unwrap_or_else(|| "untitled".into()),
            content_type: part_content_type(part),
            content: part.contents().to_vec(),
        })
        .collect();

    let thread_id = thread_id_for(&msg);
    // mailparser keeps the <> brackets on messageId.
    let header_message_id = msg.message_id().map(|mid| format!("<{}>", mid));
    let message_id = header_message_id
        .clone()
        .unwrap_or_else(|| format!("generated-{}", sha256_hex(&eml)));

    let mut from = map_addresses(msg.from());
    if from.is_empty() {
        from.push(EmailAddr {
            name: "No Sender".into(),
            address: "No Sender".into(),
        });
    }

    // Folder from client headers: X-Gmail-Labels (first label) → X-Folder → path.
    let mut final_path = source_path.to_string();
    if let Some(labels) = header_string(&msg, "x-gmail-labels") {
        if let Some(first) = labels.split(',').next() {
            final_path = first.to_string();
        }
    } else if let Some(folder) = header_string(&msg, "x-folder") {
        final_path = folder;
    }

    let received_at_ms = msg
        .date()
        .map(|d| d.to_timestamp() * 1000)
        .unwrap_or_else(now_ms);

    Ok(EmailObj {
        id: message_id,
        header_message_id,
        thread_id,
        to: map_addresses(msg.to()),
        cc: map_addresses(msg.cc()),
        bcc: map_addresses(msg.bcc()),
        from,
        subject: msg.subject().unwrap_or("").to_string(),
        body: text,
        html,
        attachments,
        received_at_ms,
        path: final_path,
        raw: eml,
    })
}

// ---------------------------------------------------------------------------
// Mbox splitting (MboxSplitter + envelope/quoting cleanup)
// ---------------------------------------------------------------------------

// (mbox splitting/unescaping lives in connectors.rs now)

// ---------------------------------------------------------------------------
// Attachment stripping (emlUtils.stripAttachmentsFromEml)
// ---------------------------------------------------------------------------

fn extract_cid_references(html: &str) -> std::collections::HashSet<String> {
    static CID: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)\bcid:([^\s"'>]+)"#).unwrap());
    CID.captures_iter(html)
        .map(|m| m[1].to_lowercase())
        .collect()
}

fn is_inline_attachment(
    part: &mail_parser::MessagePart,
    referenced_cids: &std::collections::HashSet<String>,
) -> bool {
    let cid = part.content_id().map(|c| c.trim_start_matches('<').trim_end_matches('>'));
    let disposition = part
        .content_disposition()
        .map(|d| d.ctype().to_lowercase());
    if let Some(cid) = cid {
        // mailparser's `related` flag ≈ non-attachment disposition with a CID.
        if disposition.as_deref() != Some("attachment") {
            return true;
        }
        if disposition.as_deref() == Some("inline") {
            return true;
        }
        if referenced_cids.contains(&cid.to_lowercase()) {
            return true;
        }
    }
    false
}

const HEADERS_HANDLED_BY_COMPOSER: [&str; 15] = [
    "content-type",
    "content-transfer-encoding",
    "mime-version",
    "from",
    "to",
    "cc",
    "bcc",
    "subject",
    "message-id",
    "date",
    "in-reply-to",
    "references",
    "reply-to",
    "sender",
    // nodemailer also regenerates this structural header
    "content-disposition",
];

/// Strips non-inline attachments by re-composing the message (mail-builder).
/// Returns the original buffer when there is nothing to strip.
fn strip_attachments_from_eml(eml: &[u8]) -> Vec<u8> {
    let Some(msg) = MessageParser::default().parse(eml) else {
        return eml.to_vec();
    };
    if msg.attachment_count() == 0 {
        return eml.to_vec();
    }
    let html = if msg.html_body_count() > 0 {
        msg.body_html(0).map(|c| c.to_string()).unwrap_or_default()
    } else {
        String::new()
    };
    let referenced = extract_cid_references(&html);
    let strippable = msg
        .attachments()
        .any(|p| !is_inline_attachment(p, &referenced));
    if !strippable {
        return eml.to_vec();
    }

    let mut builder = mail_builder::MessageBuilder::new();
    // Address headers passed through as raw strings (like addressToString).
    for (header, name) in [
        ("from", "From"),
        ("to", "To"),
        ("cc", "Cc"),
        ("bcc", "Bcc"),
        ("reply-to", "Reply-To"),
        ("in-reply-to", "In-Reply-To"),
        ("references", "References"),
    ] {
        if let Some(value) = msg.header_raw(header) {
            builder = builder.header(name, mail_builder::headers::raw::Raw::new(value.trim().to_string()));
        }
    }
    if let Some(subject) = msg.subject() {
        builder = builder.subject(subject.to_string());
    }
    if let Some(mid) = msg.message_id() {
        builder = builder.message_id(mid.to_string());
    }
    if let Some(date) = msg.date() {
        builder = builder.date(date.to_timestamp());
    }
    // Additional headers not handled by the composer.
    for header in msg.headers() {
        let name = header.name().to_lowercase();
        if HEADERS_HANDLED_BY_COMPOSER.contains(&name.as_str()) {
            continue;
        }
        if let Some(value) = msg.header_raw(header.name()) {
            builder = builder.header(
                header.name().to_string(),
                mail_builder::headers::raw::Raw::new(value.trim().to_string()),
            );
        }
    }
    let text = if msg.text_body_count() > 0 {
        msg.body_text(0).map(|c| c.to_string()).unwrap_or_default()
    } else {
        String::new()
    };
    if !text.is_empty() {
        builder = builder.text_body(text);
    }
    if !html.is_empty() {
        builder = builder.html_body(html.clone());
    }
    for part in msg.attachments() {
        if is_inline_attachment(part, &referenced) {
            let ctype = part_content_type(part).unwrap_or_else(|| "application/octet-stream".into());
            // Every branch of is_inline_attachment requires a CID.
            let cid = part.content_id().unwrap_or_default().to_string();
            builder = builder.inline(ctype, cid, part.contents().to_vec());
        }
    }
    builder.write_to_vec().unwrap_or_else(|_| eml.to_vec())
}

// ---------------------------------------------------------------------------
// Text extraction (helpers/textExtractor)
// ---------------------------------------------------------------------------

fn extract_text(buffer: &[u8], mime_type: &str) -> String {
    if buffer.is_empty() || mime_type.is_empty() || buffer.len() > 50 * 1024 * 1024 {
        return String::new();
    }
    if mime_type == "application/pdf" {
        return pdf_extract::extract_text_from_mem(buffer).unwrap_or_default();
    }
    if mime_type == "application/vnd.openxmlformats-officedocument.wordprocessingml.document" {
        return extract_docx_text(buffer).unwrap_or_default();
    }
    if mime_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" {
        return extract_xlsx_text(buffer).unwrap_or_default();
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
                out.push_str(&t.unescape().unwrap_or_default());
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
    user_email: &str,
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
    if existing.is_some() {
        return Ok(None);
    }

    let source_path = normalize_source_path(&email.path);
    let source_labels: Vec<String> = Vec::new(); // mbox connector sets no tags
    let mut email_tags = source_labels.clone();
    if !source_path.is_empty() && !email_tags.contains(&source_path) {
        email_tags.push(source_path.clone());
    }
    let sender_email = email.from.first().map(|a| a.address.clone()).unwrap_or_default();
    let duplicate_subject_hash = duplicate_hash(&normalize_duplicate_text(&email.subject));
    let duplicate_fuzzy_group_key = build_fuzzy_group_key(&sender_email, &duplicate_subject_hash);
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
    // Storage and DB ownership always go to the merge-group root.
    let email_path = format!(
        "open-archiver/{}-{}/emails/{}{}.eml",
        effective.name.replace(' ', "-"),
        effective.id,
        storage_path_segment,
        email.id
    );

    let eml_buffer = strip_attachments_from_eml(&email.raw);
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
    conn
        .execute(
            "INSERT INTO archived_emails (id, ingestion_source_id, user_email, thread_id, \
             message_id_header, provider_message_id, sent_at, subject, sender_name, sender_email, \
             recipients, storage_path, storage_hash_sha256, size_bytes, has_attachments, \
             source_path, source_labels, duplicate_subject_hash, duplicate_fuzzy_group_key, \
             duplicate_body_hash, duplicate_recipient_fingerprint, duplicate_attachment_fingerprint, \
             path, tags) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                archived_id,
                effective.id,
                user_email,
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
                serde_json::to_string(&source_labels).unwrap(),
                duplicate_subject_hash,
                duplicate_fuzzy_group_key,
                duplicate_body_hash,
                duplicate_recipient_fingerprint,
                Option::<String>::None,
                source_path,
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
                        "open-archiver/{}-{}/attachments/{}-{}",
                        effective.name.replace(' ', "-"),
                        effective.id,
                        unique,
                        sanitize_filename(&attachment.filename)
                    );
                    state.storage_put(&storage_path, &attachment.content)?;
                    let id = uuid();
                    conn
                        .execute(
                            "INSERT INTO attachments (id, filename, mime_type, size_bytes, \
                             content_hash_sha256, storage_path, ingestion_source_id) \
                             VALUES (?, ?, ?, ?, ?, ?, ?)",
                            rusqlite::params![
                                id,
                                attachment.filename,
                                attachment.content_type,
                                attachment.content.len() as i64,
                                attachment_hash,
                                storage_path,
                                effective.id,
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
        user_email: String,
        source_path: Option<String>,
        path: Option<String>,
        source_labels: Option<String>,
        tags: Option<String>,
        has_attachments: bool,
    }
    let row = conn
        .query_row(
            "SELECT subject, sender_email, sender_name, recipients, storage_path, user_email, \
             source_path, path, source_labels, tags, has_attachments \
             FROM archived_emails WHERE id = ?",
            [email_id],
            |r| {
                Ok(Row {
                    subject: r.get(0)?,
                    sender_email: r.get(1)?,
                    sender_name: r.get(2)?,
                    recipients: r.get(3)?,
                    storage_path: r.get(4)?,
                    user_email: r.get(5)?,
                    source_path: r.get(6)?,
                    path: r.get(7)?,
                    source_labels: r.get(8)?,
                    tags: r.get(9)?,
                    has_attachments: r.get::<_, i64>(10)? != 0,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    // Body text: stored eml → text || html || whole file as text.
    let raw = state.storage_get(&row.storage_path)?;
    let body = match MessageParser::default().parse(&raw) {
        Some(msg) => {
            let (text, html) = mailparser_text_and_html(&msg);
            if !text.is_empty() {
                text
            } else if !html.is_empty() {
                html
            } else {
                extract_text(&raw, "text/plain")
            }
        }
        None => extract_text(&raw, "text/plain"),
    };

    // Attachment contents from storage.
    let mut attachment_texts: Vec<(String, String)> = Vec::new();
    if row.has_attachments {
        let mut stmt = conn
            .prepare(
                "SELECT a.filename, a.mime_type, a.storage_path FROM email_attachments ea \
                 INNER JOIN attachments a ON ea.attachment_id = a.id WHERE ea.email_id = ?",
            )
            .map_err(|e| e.to_string())?;
        let atts: Vec<(String, Option<String>, String)> = stmt
            .query_map([email_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        for (filename, mime, storage_path) in atts {
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
    let labels: Vec<String> = row
        .source_labels
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
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
    let source_path_value = row
        .source_path
        .clone()
        .filter(|s| !s.is_empty())
        .or(row.path.clone())
        .unwrap_or_default();
    let mut meta_parts: Vec<String> = vec![sanitize_text(&row.user_email), sanitize_text(&source_path_value)];
    meta_parts.extend(labels.iter().map(|l| sanitize_text(l)));
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
