//! Mbox/eml input discovery + email iteration, driven by the source's
//! localFilePath (file or directory). Apple Mail .emlx messages inside .mbox
//! bundles are supported.

use crate::ingest::{parse_message, EmailObj};
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
        let rel = path
            .strip_prefix(import_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| name.clone());
        if path.is_dir() {
            find_local_inputs(import_root, &path, out)?;
        } else if path.is_file() && is_mbox_path(&name) {
            out.push(MboxInput {
                file_path: path.to_string_lossy().to_string(),
                source_path: to_source_path(&rel),
                is_emlx: false,
                is_eml: false,
            });
        } else if path.is_file()
            && is_emlx_path(&name)
            && (is_mbox_path(&import_root.to_string_lossy())
                || rel.split(['\\', '/']).any(is_mbox_path))
        {
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
    if !is_mbox_path(local) && !is_eml_path(local) {
        return Err("Provided local file is not in the MBOX or EML format.".into());
    }
    Ok(vec![MboxInput {
        file_path: local.to_string(),
        source_path: String::new(),
        is_emlx: false,
        is_eml: is_eml_path(local),
    }])
}

/// getDisplayName → the constructed <name>@mbox.local mailbox user.
pub fn mbox_user_email(provider_config: &Value) -> String {
    let display = mbox_display_name(provider_config);
    format!("{}@mbox.local", display.replace(' ', ".").to_lowercase())
}

fn mbox_display_name(provider_config: &Value) -> String {
    if let Some(local) = provider_config.get("localFilePath").and_then(|v| v.as_str()) {
        let base = Path::new(local)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let base = MBOX_EXT.replace(&base, "").to_string();
        return base.strip_suffix(".eml").unwrap_or(&base).to_string();
    }
    format!("mbox-import-{}", crate::search::now_ms())
}

fn extract_emlx_message(buffer: &[u8], file_path: &str) -> Result<Vec<u8>, String> {
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
    Ok(buffer[start..end].to_vec())
}

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
        let bytes = match std::fs::read(&input.file_path).map_err(|e| e.to_string()) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[reader] failed to read input {}: {e}", input.file_path);
                continue; // Node logs + skips the input
            }
        };
        if input.is_emlx {
            match extract_emlx_message(&bytes, &input.file_path) {
                Ok(eml) => match parse_message(eml, &input.source_path) {
                    Ok(email) => handle(email),
                    Err(e) => eprintln!("[reader] emlx parse failed: {e}"),
                },
                Err(e) => eprintln!("[reader] {e}"),
            }
            continue;
        }
        if input.is_eml {
            match parse_message(bytes, &input.source_path) {
                Ok(email) => handle(email),
                Err(e) => eprintln!("[reader] eml parse failed: {e}"),
            }
            continue;
        }
        for chunk in split_mbox(&bytes) {
            let eml = unescape_mbox_quoting(strip_mbox_envelope(&chunk));
            match parse_message(eml, &input.source_path) {
                Ok(email) => handle(email),
                Err(e) => eprintln!("[reader] mbox message parse failed: {e}"),
            }
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

    #[test]
    fn path_predicates_and_derivations() {
        assert!(is_mbox_path("X.MBOX") && is_emlx_path("a.EMLX") && is_eml_path("b.Eml"));
        assert!(!is_mbox_path("x.txt"));
        assert_eq!(to_eml_source_path("a/b/c.eml"), "a/b");
        assert_eq!(to_eml_source_path("root.eml"), "");
        assert_eq!(to_apple_mail_source_path("Parent.mbox/Data/msg.emlx"), "Parent");
    }

    #[test]
    fn mbox_display_name_and_user_email() {
        let c = serde_json::json!({ "localFilePath": "/x/My Mail.mbox" });
        assert_eq!(mbox_display_name(&c), "My Mail");
        assert_eq!(mbox_user_email(&c), "my.mail@mbox.local");
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
        assert_eq!(extract_emlx_message(b"5\nhello world extra", "x.emlx").unwrap(), b"hello");
    }

    #[test]
    fn extract_emlx_rejects_overflow_and_truncation() {
        // an overflowing declared length must error, not panic
        assert!(extract_emlx_message(b"18446744073709551615\nx", "x.emlx").is_err());
        assert!(extract_emlx_message(b"100\nshort", "x.emlx").is_err());
        assert!(extract_emlx_message(b"nolength", "x.emlx").is_err());
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
