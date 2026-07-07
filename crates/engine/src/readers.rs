//! Mbox/eml input discovery + email iteration, driven by the source's
//! localFilePath (file or directory). Two Apple Mail bundle layouts inside a
//! `.mbox` package are supported: live-store `.emlx` messages
//! (`Foo.mbox/Data/N/Messages/*.emlx`) and "Export Mailbox" packages, which
//! hold a single raw Unix mbox file named literally `mbox` (`Foo.mbox/mbox`,
//! alongside `Info.plist` / `Table of Contents` index files that we ignore).

use crate::ingest::{parse_message, EmailObj, EmlxMeta};
use std::sync::LazyLock;
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};

static MBOX_EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.mbox$").unwrap());

#[derive(Clone)]
pub struct MboxInput {
    pub file_path: String,
    pub source_path: String,
    pub is_emlx: bool,
    /// A plain RFC 5322 .eml file: the whole file is one message.
    pub is_eml: bool,
}

fn is_mbox_path(p: &str) -> bool {
    p.to_lowercase().ends_with(".mbox")
}
/// Apple Mail's "Export Mailbox" writes each mailbox as a `Foo.mbox/` package
/// whose message data is a single raw Unix mbox file named literally `mbox`
/// (no extension). Matched only inside a `.mbox` bundle (see `find_local_inputs`)
/// so a stray file called `mbox` elsewhere is never imported.
fn is_bare_mbox_file(name: &str) -> bool {
    name.eq_ignore_ascii_case("mbox")
}
fn is_emlx_path(p: &str) -> bool {
    p.to_lowercase().ends_with(".emlx")
}
fn is_eml_path(p: &str) -> bool {
    p.to_lowercase().ends_with(".eml")
}

/// Source path for a loose .eml: its parent folders relative to the import
/// root (the filename itself is not a mailbox).
fn to_eml_source_path(rel: &str) -> String {
    let segments: Vec<&str> = rel.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    segments[..segments.len().saturating_sub(1)].join("/")
}

fn to_source_path(file_path: &str) -> String {
    // Strip `.mbox` from EVERY segment (the regex is `$`-anchored), so
    // "Parent.mbox/child.mbox" → "Parent/child" like the Apple-Mail variant.
    file_path
        .split(['\\', '/'])
        .map(|seg| MBOX_EXT.replace(seg, "").to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn to_apple_mail_source_path(file_path: &str) -> String {
    file_path
        .split(['\\', '/'])
        .filter(|seg| is_mbox_path(seg))
        .map(|seg| MBOX_EXT.replace(seg, "").to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn find_local_inputs(import_root: &Path, dir: &Path, out: &mut Vec<MboxInput>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // Don't follow symlinks: a self-referential link would recurse forever
        // (stack overflow / ENAMETOOLONG that aborts the import), and a link
        // pointing outside the tree would import files from outside the selected
        // root. entry.file_type() reports the link itself, without traversing it.
        if entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
            continue;
        }
        let rel = path
            .strip_prefix(import_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| name.clone());
        // Skip macOS AppleDouble sidecars (._*): resource-fork/metadata files
        // that appear when Mail data is copied onto a non-HFS+ filesystem. They
        // carry the real item's name (._x.emlx, ._x.mbox) but hold no email, so
        // they'd otherwise be imported and fail to parse (a "missing length
        // line" error per hidden sidecar).
        if name.starts_with("._") {
            continue;
        }
        // True when this entry lives inside an Apple Mail `.mbox` package —
        // either the import root itself is one, or some ancestor segment ends
        // in `.mbox`. Gates the two bundle-only layouts (`.emlx` messages and
        // the bare `mbox` export file) so neither is picked up loose on disk.
        let inside_mbox_bundle = is_mbox_path(&import_root.to_string_lossy())
            || rel.split(['\\', '/']).any(is_mbox_path);
        if path.is_dir() {
            find_local_inputs(import_root, &path, out)?;
        } else if path.is_file() && is_mbox_path(&name) {
            out.push(MboxInput {
                file_path: path.to_string_lossy().to_string(),
                source_path: to_source_path(&rel),
                is_emlx: false,
                is_eml: false,
            });
        } else if path.is_file() && is_bare_mbox_file(&name) && inside_mbox_bundle {
            // Apple "Export Mailbox" package: Foo.mbox/mbox is a classic Unix
            // mbox. The bundle's `.mbox` segment(s) become the folder path (the
            // `mbox` filename is not itself a mailbox), matching the .emlx case.
            out.push(MboxInput {
                file_path: path.to_string_lossy().to_string(),
                source_path: to_apple_mail_source_path(&rel),
                is_emlx: false,
                is_eml: false,
            });
        } else if path.is_file() && is_emlx_path(&name) && inside_mbox_bundle {
            out.push(MboxInput {
                file_path: path.to_string_lossy().to_string(),
                source_path: to_apple_mail_source_path(&rel),
                is_emlx: true,
                is_eml: false,
            });
        } else if path.is_file() && is_eml_path(&name) {
            out.push(MboxInput {
                file_path: path.to_string_lossy().to_string(),
                source_path: to_eml_source_path(&rel),
                is_emlx: false,
                is_eml: true,
            });
        }
    }
    Ok(())
}

/// getMboxInputs — resolves the source's local path (file or directory)
/// into the list of mbox/eml/emlx inputs to import.
pub fn get_mbox_inputs(provider_config: &Value) -> Result<Vec<MboxInput>, String> {
    let local = provider_config.get("localFilePath").and_then(|v| v.as_str()).unwrap_or("");
    if local.is_empty() {
        return Err("Mbox file or folder path not provided.".into());
    }

    let path = PathBuf::from(local);
    let meta = std::fs::metadata(&path)
        .map_err(|_| format!("Mbox file or folder not found at path: {local}"))?;
    if meta.is_dir() {
        let mut inputs = Vec::new();
        find_local_inputs(&path, &path, &mut inputs)?;
        if inputs.is_empty() {
            return Err(format!(
                "No mbox, .eml, or Apple Mail message files found under directory: {local}"
            ));
        }
        inputs.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        return Ok(inputs);
    }
    if !meta.is_file() {
        return Err(format!("Mbox path is not a file or directory: {local}"));
    }
    // A directly-selected inner `mbox` file counts only when its parent is a
    // `.mbox` export package (Foo.mbox/mbox), so a random `mbox` file is still
    // rejected with the same error as before.
    let bare_mbox_in_bundle = is_bare_mbox_file(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
        && path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| is_mbox_path(&n.to_string_lossy()))
            .unwrap_or(false);
    if !is_mbox_path(local) && !is_eml_path(local) && !bare_mbox_in_bundle {
        return Err("Provided local file is not in the MBOX or EML format.".into());
    }
    Ok(vec![MboxInput {
        file_path: local.to_string(),
        source_path: String::new(),
        is_emlx: false,
        is_eml: is_eml_path(local),
    }])
}

/// The import's display name (from the mbox file name), used as its Import Source.
pub fn mbox_import_source(provider_config: &Value) -> String {
    mbox_display_name(provider_config)
}

fn mbox_display_name(provider_config: &Value) -> String {
    if let Some(local) = provider_config.get("localFilePath").and_then(|v| v.as_str()) {
        let path = Path::new(local);
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // Foo.mbox/mbox: the bare export file is named after its package, not
        // the generic "mbox" filename.
        let base = if is_bare_mbox_file(&file) {
            path.parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(file)
        } else {
            file
        };
        let base = MBOX_EXT.replace(&base, "").to_string();
        return base.strip_suffix(".eml").unwrap_or(&base).to_string();
    }
    format!("mbox-import-{}", crate::search::now_ms())
}

fn extract_emlx_message(buffer: &[u8], file_path: &str) -> Result<(Vec<u8>, EmlxMeta), String> {
    let newline = buffer
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| format!("Invalid Apple Mail EMLX file (missing length line): {file_path}"))?;
    let length_line = String::from_utf8_lossy(&buffer[..newline]).trim().to_string();
    let message_length: usize = length_line
        .parse()
        .map_err(|_| format!("Invalid Apple Mail EMLX file (invalid length): {file_path}"))?;
    let start = newline + 1;
    // checked_add: a bogus/overflowing declared length must error, not panic.
    let end = start
        .checked_add(message_length)
        .ok_or_else(|| format!("Invalid Apple Mail EMLX file (truncated message): {file_path}"))?;
    if message_length == 0 || end > buffer.len() {
        return Err(format!("Invalid Apple Mail EMLX file (truncated message): {file_path}"));
    }
    // Everything after the message is Apple's plist metadata trailer — parse it
    // as a fallback for messages whose own RFC-822 headers are missing.
    let meta = parse_emlx_plist(&buffer[end..]);
    Ok((buffer[start..end].to_vec(), meta))
}

/// Extract date-sent / subject / sender / to from an emlx plist trailer. The
/// trailer is a flat `<key>NAME</key><type>VALUE</type>` dict, so a direct
/// scan is simpler (and avoids quick-xml 0.41 splitting text at entity refs).
/// Best effort: anything unparseable just leaves the field empty.
fn parse_emlx_plist(trailer: &[u8]) -> EmlxMeta {
    let mut meta = EmlxMeta::default();
    if trailer.is_empty() {
        return meta;
    }
    let text = String::from_utf8_lossy(trailer);
    if let Some(secs) = plist_value(&text, "date-sent").and_then(|v| v.trim().parse::<f64>().ok()) {
        meta.date_sent_ms = Some((secs * 1000.0) as i64);
    }
    meta.subject = plist_string(&text, "subject");
    meta.sender = plist_string(&text, "sender");
    meta.to = plist_string(&text, "to");
    meta
}

/// Raw inner text of the value element immediately following `<key>KEY</key>`
/// (entities left intact, e.g. `Name &lt;a@b&gt;`).
fn plist_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let key_tag = format!("<key>{key}</key>");
    let after = text[text.find(&key_tag)? + key_tag.len()..].trim_start();
    let open_end = after.find('>')?; // end of the value element's opening tag
    let inner = &after[open_end + 1..];
    // Entities use `&lt;`/`&gt;`, so the first literal `<` is the closing tag.
    let close = inner.find('<')?;
    Some(&inner[..close])
}

/// A plist string value for `key`, XML-unescaped; None if absent or blank.
fn plist_string(text: &str, key: &str) -> Option<String> {
    let raw = plist_value(text, key)?;
    let unescaped = quick_xml::escape::unescape(raw)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| raw.to_string());
    let trimmed = unescaped.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Kept as the reference splitter that `for_each_mbox_message` is tested against
/// (the import path itself now streams instead of buffering the whole file).
#[cfg(test)]
fn split_mbox(buffer: &[u8]) -> Vec<Vec<u8>> {
    let delim = b"\nFrom ";
    let mut out = Vec::new();
    let mut rest = buffer;
    while let Some(pos) = rest.windows(delim.len()).position(|w| w == delim) {
        if pos > 0 {
            out.push(rest[..pos].to_vec());
        }
        rest = &rest[pos + 1..];
    }
    if !rest.is_empty() {
        out.push(rest.to_vec());
    }
    out
}

fn strip_mbox_envelope(buffer: &[u8]) -> &[u8] {
    if buffer.starts_with(b"From ") {
        if let Some(nl) = buffer.iter().position(|b| *b == b'\n') {
            return &buffer[nl + 1..];
        }
    }
    buffer
}

fn unescape_mbox_quoting(buffer: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(buffer.len());
    for line in buffer.split_inclusive(|b| *b == b'\n') {
        let mut stripped = 0usize;
        while stripped < line.len() && line[stripped] == b'>' {
            stripped += 1;
        }
        if stripped > 0 && line[stripped..].starts_with(b"From ") {
            out.extend_from_slice(&line[1..]);
        } else {
            out.extend_from_slice(line);
        }
    }
    out
}

/// Streams an mbox file message-by-message without loading the whole file, so a
/// multi-GB archive never has to fit in memory (only one message at a time).
/// Each yielded chunk is byte-identical to `split_mbox`'s output: the message
/// including its leading "From " envelope, minus the single '\n' that precedes
/// the next "From " line.
fn for_each_mbox_message(
    path: &str,
    mut on_message: impl FnMut(Vec<u8>),
) -> Result<(), String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut msg: Vec<u8> = Vec::new();
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            break; // EOF
        }
        // A "From " at the start of a line begins a new message. The first
        // message's leading envelope is not a boundary (msg is still empty there).
        if line.starts_with(b"From ") && !msg.is_empty() {
            if msg.last() == Some(&b'\n') {
                msg.pop(); // the '\n' before "From " is the delimiter, not content
            }
            on_message(std::mem::take(&mut msg));
        }
        msg.extend_from_slice(&line);
    }
    if !msg.is_empty() {
        on_message(msg);
    }
    Ok(())
}

/// fetchEmails — iterates every message of every input.
pub fn for_each_email(
    provider_config: &Value,
    mut handle: impl FnMut(EmailObj),
) -> Result<(), String> {
    let inputs = get_mbox_inputs(provider_config)?;

    // No reader-level content dedup: the same message can appear in several
    // folders (Apple Mail bundles, copied .eml), each with a distinct
    // source_path. process_email dedups by message-id within the merge group
    // AND merges each duplicate's folder into the surviving email's tags, so a
    // message stays findable under every folder it appeared in. (Deduping here
    // would drop those extra folder tags — the mbox path never did.)
    for input in &inputs {
        // .emlx / .eml are single, small messages — read them whole. An .mbox can
        // be many GB, so it is streamed below rather than read into memory.
        if input.is_emlx || input.is_eml {
            let bytes = match std::fs::read(&input.file_path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[reader] failed to read input {}: {e}", input.file_path);
                    continue; // log and skip this input
                }
            };
            if input.is_emlx {
                match extract_emlx_message(&bytes, &input.file_path) {
                    Ok((eml, meta)) => match parse_message(eml, &input.source_path, Some(&meta)) {
                        Ok(email) => handle(email),
                        Err(e) => eprintln!("[reader] emlx parse failed: {e}"),
                    },
                    Err(e) => eprintln!("[reader] {e}"),
                }
            } else {
                match parse_message(bytes, &input.source_path, None) {
                    Ok(email) => handle(email),
                    Err(e) => eprintln!("[reader] eml parse failed: {e}"),
                }
            }
            continue;
        }

        let read_result = for_each_mbox_message(&input.file_path, |chunk| {
            let eml = unescape_mbox_quoting(strip_mbox_envelope(&chunk));
            match parse_message(eml, &input.source_path, None) {
                Ok(email) => handle(email),
                Err(e) => eprintln!("[reader] mbox message parse failed: {e}"),
            }
        });
        if let Err(e) = read_result {
            eprintln!("[reader] failed to read input {}: {e}", input.file_path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_source_path_strips_mbox_per_segment() {
        assert_eq!(to_source_path("Parent.mbox/child.mbox"), "Parent/child");
        assert_eq!(to_source_path("/a/b.mbox"), "a/b");
        assert_eq!(to_source_path("plain"), "plain");
    }

    #[test]
    fn split_mbox_splits_on_from_lines() {
        let msgs = split_mbox(b"From a\n\nbody one\nFrom b\n\nbody two");
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].starts_with(b"From a"));
        assert!(msgs[1].starts_with(b"From b"));
        assert_eq!(split_mbox(b"From a\n\nbody").len(), 1);
    }

    // The streaming reader must yield byte-identical message chunks to split_mbox,
    // so switching to streaming can't change stored bytes / storage hashes.
    #[test]
    fn streaming_mbox_matches_split_mbox() {
        for data in [
            &b"From a\n\nbody one\nFrom b\n\nbody two\n"[..],
            &b"From a\n\nbody one\nFrom b\n\nbody two"[..], // no trailing newline
            &b"From only\n\nsingle message body\n"[..],
            &b">From quoted\nFrom real\n\nbody"[..], // ">From" is not a boundary
        ] {
            let dir = std::env::temp_dir()
                .join(format!("pea-mboxstream-{}-{}", std::process::id(), data.len()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("s.mbox");
            std::fs::write(&path, data).unwrap();
            let mut streamed: Vec<Vec<u8>> = Vec::new();
            for_each_mbox_message(path.to_str().unwrap(), |c| streamed.push(c)).unwrap();
            std::fs::remove_dir_all(&dir).ok();
            assert_eq!(streamed, split_mbox(data), "streaming differs from split_mbox for {data:?}");
        }
    }

    #[test]
    fn path_predicates_and_derivations() {
        assert!(is_mbox_path("X.MBOX") && is_emlx_path("a.EMLX") && is_eml_path("b.Eml"));
        assert!(!is_mbox_path("x.txt"));
        assert!(is_bare_mbox_file("mbox") && is_bare_mbox_file("MBOX"));
        assert!(!is_bare_mbox_file("mbox.txt") && !is_bare_mbox_file("A.mbox"));
        assert_eq!(to_eml_source_path("a/b/c.eml"), "a/b");
        assert_eq!(to_eml_source_path("root.eml"), "");
        assert_eq!(to_apple_mail_source_path("Parent.mbox/Data/msg.emlx"), "Parent");
        // The bare `mbox` filename is not a `.mbox` segment, so it never becomes
        // a folder — only the enclosing package name(s) do.
        assert_eq!(to_apple_mail_source_path("Parent.mbox/mbox"), "Parent");
        assert_eq!(to_apple_mail_source_path("A.mbox/Sub.mbox/mbox"), "A/Sub");
    }

    #[test]
    fn apple_export_mailbox_bare_mbox_is_discovered() {
        // Apple Mail "Export Mailbox" package layout: Foo.mbox/mbox is the real
        // Unix mbox, sitting next to Info.plist / Table of Contents index files
        // that must be ignored. Reproduces selecting a parent folder holding two
        // exported mailboxes, one nested inside another package.
        let dir = std::env::temp_dir().join(format!("pea-apple-export-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let archive = dir.join("Archive.mbox");
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(archive.join("mbox"), b"From a\n\nbody").unwrap();
        std::fs::write(archive.join("Info.plist"), b"<plist/>").unwrap();
        std::fs::write(archive.join("Table of Contents"), b"junk").unwrap();
        let nested = dir.join("Parent.mbox/Child.mbox");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("mbox"), b"From b\n\nbody").unwrap();

        let mut inputs = Vec::new();
        find_local_inputs(&dir, &dir, &mut inputs).unwrap();
        inputs.sort_by(|a, b| a.file_path.cmp(&b.file_path));

        assert_eq!(inputs.len(), 2, "only the two `mbox` files, not plist/TOC");
        assert!(
            inputs.iter().all(|i| !i.is_emlx && !i.is_eml),
            "bare mbox is treated as a raw mbox stream"
        );
        let top = inputs.iter().find(|i| i.file_path.contains("Archive.mbox")).unwrap();
        assert_eq!(top.source_path, "Archive", "package name becomes the folder");
        let child = inputs.iter().find(|i| i.file_path.contains("Parent.mbox")).unwrap();
        assert_eq!(child.source_path, "Parent/Child", "nested packages nest the folder");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bare_mbox_outside_a_package_is_ignored() {
        // A file literally named `mbox` that is NOT inside a `.mbox` package is
        // not email data — importing it would fail to parse. Only .eml counts.
        let dir = std::env::temp_dir().join(format!("pea-bare-mbox-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mbox"), b"not in a bundle").unwrap();
        std::fs::write(dir.join("keep.eml"), b"From: a@x\r\n\r\nhi").unwrap();

        let mut inputs = Vec::new();
        find_local_inputs(&dir, &dir, &mut inputs).unwrap();

        assert_eq!(inputs.len(), 1, "the stray `mbox` file is skipped");
        assert!(inputs[0].is_eml && inputs[0].file_path.ends_with("keep.eml"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn selecting_the_export_package_directly_names_it_after_the_package() {
        // Selecting the Foo.mbox package (a directory) discovers its inner `mbox`
        // with an empty folder path (the package itself is the import root).
        let dir = std::env::temp_dir().join(format!("pea-export-pkg-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let pkg = dir.join("Sent Messages.mbox");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("mbox"), b"From a\n\nbody").unwrap();

        let cfg = serde_json::json!({ "localFilePath": pkg.to_string_lossy() });
        let inputs = get_mbox_inputs(&cfg).unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].source_path, "", "package is the root, no folder prefix");
        assert_eq!(mbox_display_name(&cfg), "Sent Messages");

        // Selecting the inner `mbox` file directly is also accepted and named
        // after its package rather than the generic "mbox".
        let inner = serde_json::json!({ "localFilePath": pkg.join("mbox").to_string_lossy() });
        assert_eq!(get_mbox_inputs(&inner).unwrap().len(), 1);
        assert_eq!(mbox_display_name(&inner), "Sent Messages");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn export_package_messages_import_end_to_end() {
        // The bare `mbox` file is streamed like any Unix mbox: both messages come
        // through, tagged with the package's folder name.
        let dir = std::env::temp_dir().join(format!("pea-export-e2e-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let pkg = dir.join("Archive.mbox");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("mbox"),
            b"From a@x Thu Jan  1 00:00:00 2020\r\n\
              From: a@x\r\nSubject: One\r\nMessage-ID: <a@x>\r\n\r\nbody one\r\n\
              From b@x Thu Jan  1 00:00:00 2020\r\n\
              From: b@x\r\nSubject: Two\r\nMessage-ID: <b@x>\r\n\r\nbody two\r\n",
        )
        .unwrap();

        let cfg = serde_json::json!({ "localFilePath": dir.to_string_lossy() });
        let mut emails = Vec::new();
        for_each_email(&cfg, |e| emails.push(e)).unwrap();
        emails.sort_by(|a, b| a.subject.cmp(&b.subject));

        assert_eq!(emails.len(), 2, "both mbox messages imported");
        assert_eq!(emails[0].subject, "One");
        assert_eq!(emails[1].subject, "Two");
        assert!(emails.iter().all(|e| e.path == "Archive"), "folder tag is the package name");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mbox_display_name_and_import_source() {
        let c = serde_json::json!({ "localFilePath": "/x/My Mail.mbox" });
        assert_eq!(mbox_display_name(&c), "My Mail");
        assert_eq!(mbox_import_source(&c), "My Mail");
    }

    #[test]
    fn envelope_strip_and_from_unquoting() {
        assert_eq!(strip_mbox_envelope(b"From x\nrest"), b"rest");
        assert_eq!(strip_mbox_envelope(b"no envelope"), b"no envelope");
        assert_eq!(unescape_mbox_quoting(b">From foo\nbody"), b"From foo\nbody");
        assert_eq!(unescape_mbox_quoting(b">>From x\n"), b">From x\n");
        assert_eq!(unescape_mbox_quoting(b">quoted text\n"), b">quoted text\n");
    }

    #[test]
    fn extract_emlx_reads_declared_length() {
        assert_eq!(extract_emlx_message(b"5\nhello world extra", "x.emlx").unwrap().0, b"hello");
    }

    #[test]
    fn extract_emlx_rejects_overflow_and_truncation() {
        // an overflowing declared length must error, not panic
        assert!(extract_emlx_message(b"18446744073709551615\nx", "x.emlx").is_err());
        assert!(extract_emlx_message(b"100\nshort", "x.emlx").is_err());
        assert!(extract_emlx_message(b"nolength", "x.emlx").is_err());
    }

    #[test]
    fn emlx_plist_trailer_backfills_missing_headers() {
        // Reproduces the real corrupt message: a headerless body, but Apple's
        // plist trailer still carries the date/subject/sender.
        let body = "\n\nCiao ragazzi, come vanno le vacanze?\n";
        let plist = "<?xml version=\"1.0\"?><plist><dict>\
            <key>date-sent</key><real>1256610359</real>\
            <key>subject</key><string>Gruppi di conversazione</string>\
            <key>sender</key><string>Ivana Di Siena &lt;ivana@x.edu&gt;</string>\
            </dict></plist>";
        let emlx = format!("{}\n{}{}", body.len(), body, plist);

        let (eml, meta) = extract_emlx_message(emlx.as_bytes(), "x.emlx").unwrap();
        assert_eq!(meta.date_sent_ms, Some(1256610359000));
        assert_eq!(meta.subject.as_deref(), Some("Gruppi di conversazione"));
        assert_eq!(meta.sender.as_deref(), Some("Ivana Di Siena <ivana@x.edu>"));

        // The headerless message backfills every missing field from the plist.
        let email = parse_message(eml, "Folder", Some(&meta)).unwrap();
        assert_eq!(email.subject, "Gruppi di conversazione");
        assert_eq!(email.received_at_ms, 1256610359000);
        assert_eq!(email.from[0].address, "ivana@x.edu");
        assert_eq!(email.from[0].name, "Ivana Di Siena");
    }

    #[test]
    fn appledouble_sidecars_are_skipped() {
        // Reproduces the real Apple Mail store copied to a non-HFS+ disk: every
        // real message has a hidden `._<name>` AppleDouble twin, and the mailbox
        // dir has one too. Only the real .emlx must be discovered.
        let dir = std::env::temp_dir().join(format!("pea-appledouble-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let messages = dir.join("Oberlin.mbox/Data/2/Messages");
        std::fs::create_dir_all(&messages).unwrap();
        std::fs::write(messages.join("32590.emlx"), b"5\nhello").unwrap();
        std::fs::write(messages.join("._32590.emlx"), b"\x00\x05\x16\x07junk").unwrap();
        std::fs::write(dir.join("._Oberlin.mbox"), b"\x00\x05\x16\x07junk").unwrap();

        let mut inputs = Vec::new();
        find_local_inputs(&dir, &dir, &mut inputs).unwrap();

        assert_eq!(inputs.len(), 1, "only the real .emlx is imported, sidecars skipped");
        assert!(inputs[0].is_emlx && inputs[0].file_path.ends_with("32590.emlx"));
        assert!(!inputs[0].file_path.contains("._"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn folder_of_loose_eml_files_is_discovered() {
        let dir = std::env::temp_dir().join(format!("pea-eml-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("Saved")).unwrap();
        std::fs::write(dir.join("a.eml"), b"From: a@x\r\n\r\nhi").unwrap();
        std::fs::write(dir.join("Saved/b.eml"), b"From: b@x\r\n\r\nyo").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();

        let mut inputs = Vec::new();
        find_local_inputs(&dir, &dir, &mut inputs).unwrap();
        inputs.sort_by(|a, b| a.file_path.cmp(&b.file_path));

        assert_eq!(inputs.len(), 2, "both .eml files found, txt ignored");
        assert!(inputs.iter().all(|i| i.is_eml && !i.is_emlx));
        let nested = inputs
            .iter()
            .find(|i| i.file_path.ends_with("b.eml"))
            .unwrap();
        assert_eq!(nested.source_path, "Saved", "folder becomes the source path");
        let root = inputs
            .iter()
            .find(|i| i.file_path.ends_with("a.eml"))
            .unwrap();
        assert_eq!(root.source_path, "", "root-level file has no folder path");

        std::fs::remove_dir_all(&dir).ok();
    }
}
