//! Port of MboxConnector's input discovery + email iteration, driven by the
//! decrypted source credentials (localFilePath file-or-directory, uploadedFiles
//! in storage, legacy uploadedFilePath). Apple Mail .emlx messages inside .mbox
//! bundles are supported like Node.

use crate::ingest::{parse_message, EmailObj};
use crate::state::AppState;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

static MBOX_EXT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\.mbox$").unwrap());

#[derive(Clone)]
pub struct MboxInput {
    pub file_path: String,
    pub source_path: String,
    pub is_local: bool,
    pub is_emlx: bool,
}

fn is_mbox_path(p: &str) -> bool {
    p.to_lowercase().ends_with(".mbox")
}
fn is_emlx_path(p: &str) -> bool {
    p.to_lowercase().ends_with(".emlx")
}

fn to_source_path(file_path: &str) -> String {
    MBOX_EXT
        .replace(file_path, "")
        .split(['\\', '/'])
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
                is_local: true,
                is_emlx: false,
            });
        } else if path.is_file()
            && is_emlx_path(&name)
            && (is_mbox_path(&import_root.to_string_lossy())
                || rel.split(['\\', '/']).any(is_mbox_path))
        {
            out.push(MboxInput {
                file_path: path.to_string_lossy().to_string(),
                source_path: to_apple_mail_source_path(&rel),
                is_local: true,
                is_emlx: true,
            });
        }
    }
    Ok(())
}

/// getMboxInputs — validation errors carry the same messages Node throws
/// (surfaced as the 400 body by the create endpoint's testConnection).
pub fn get_mbox_inputs(state: &AppState, credentials: &Value) -> Result<Vec<MboxInput>, String> {
    let local = credentials.get("localFilePath").and_then(|v| v.as_str()).unwrap_or("");
    let uploaded_single = credentials
        .get("uploadedFilePath")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let uploaded_files: Vec<&Value> = credentials
        .get("uploadedFiles")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    if local.is_empty() && uploaded_single.is_empty() && uploaded_files.is_empty() {
        return Err("Mbox file or folder path not provided.".into());
    }

    if !local.is_empty() {
        let path = PathBuf::from(local);
        let meta = std::fs::metadata(&path).map_err(|_| {
            format!("Mbox file or folder not found inside the OpenArchiver server at path: {local}")
        })?;
        if meta.is_dir() {
            let mut inputs = Vec::new();
            find_local_inputs(&path, &path, &mut inputs)?;
            if inputs.is_empty() {
                return Err(format!(
                    "No mbox files or Apple Mail messages found under directory: {local}"
                ));
            }
            inputs.sort_by(|a, b| a.file_path.cmp(&b.file_path));
            return Ok(inputs);
        }
        if !meta.is_file() {
            return Err(format!("Mbox path is not a file or directory: {local}"));
        }
        if !is_mbox_path(local) {
            return Err("Provided local file is not in the MBOX format.".into());
        }
        return Ok(vec![MboxInput {
            file_path: local.to_string(),
            source_path: String::new(),
            is_local: true,
            is_emlx: false,
        }]);
    }

    if !uploaded_files.is_empty() {
        for file in &uploaded_files {
            let file_name = file.get("fileName").and_then(|v| v.as_str()).unwrap_or("");
            if !is_mbox_path(file_name) && !is_emlx_path(file_name) {
                return Err(format!(
                    "Uploaded file is not an MBOX or Apple Mail EMLX file: {file_name}"
                ));
            }
            let file_path = file.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
            if !state.storage_root().join(file_path).is_file() {
                return Err(format!("Uploaded Mbox file not found: {file_name}"));
            }
        }
        let use_names_as_paths = uploaded_files.len() > 1;
        return Ok(uploaded_files
            .iter()
            .map(|file| {
                let file_name = file.get("fileName").and_then(|v| v.as_str()).unwrap_or("");
                let file_path = file.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
                let relative = file
                    .get("relativePath")
                    .and_then(|v| v.as_str())
                    .filter(|r| !r.is_empty())
                    .unwrap_or(file_name);
                let is_emlx = is_emlx_path(file_name);
                MboxInput {
                    file_path: file_path.to_string(),
                    source_path: if is_emlx {
                        to_apple_mail_source_path(relative)
                    } else if use_names_as_paths {
                        to_source_path(file_name)
                    } else {
                        String::new()
                    },
                    is_local: false,
                    is_emlx,
                }
            })
            .collect());
    }

    if !is_mbox_path(uploaded_single) {
        return Err("Provided file is not in the MBOX format.".into());
    }
    if !state.storage_root().join(uploaded_single).is_file() {
        return Err(
            "Uploaded Mbox file not found. The upload may not have finished yet, or it failed."
                .into(),
        );
    }
    Ok(vec![MboxInput {
        file_path: uploaded_single.to_string(),
        source_path: String::new(),
        is_local: false,
        is_emlx: false,
    }])
}

/// getDisplayName → the constructed <name>@mbox.local mailbox user.
pub fn mbox_user_email(credentials: &Value) -> String {
    let display = mbox_display_name(credentials);
    format!("{}@mbox.local", display.replace(' ', ".").to_lowercase())
}

fn mbox_display_name(credentials: &Value) -> String {
    if let Some(files) = credentials.get("uploadedFiles").and_then(|v| v.as_array()) {
        if files.len() == 1 {
            if let Some(name) = files[0].get("fileName").and_then(|v| v.as_str()) {
                return name.to_string();
            }
        }
        if files.len() > 1 {
            return format!("{}-file-mbox-import", files.len());
        }
    }
    if let Some(name) = credentials.get("uploadedFileName").and_then(|v| v.as_str()) {
        return name.to_string();
    }
    if let Some(local) = credentials.get("localFilePath").and_then(|v| v.as_str()) {
        let base = Path::new(local)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        return MBOX_EXT.replace(&base, "").to_string();
    }
    format!("mbox-import-{}", crate::search::now_ms())
}

fn read_input(state: &AppState, input: &MboxInput) -> Result<Vec<u8>, String> {
    if input.is_local {
        std::fs::read(&input.file_path).map_err(|e| e.to_string())
    } else {
        state.storage_get(&input.file_path)
    }
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
    let end = start + message_length;
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

/// fetchEmails — iterates every message of every input; uploaded files are
/// deleted from storage afterwards (like the Node connector's finally block).
pub fn for_each_email(
    state: &AppState,
    credentials: &Value,
    mut handle: impl FnMut(EmailObj),
) -> Result<(), String> {
    let inputs = get_mbox_inputs(state, credentials)?;
    let mut seen_apple: HashSet<String> = HashSet::new();

    for input in &inputs {
        let bytes = match read_input(state, input) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[connector] failed to read input {}: {e}", input.file_path);
                continue; // Node logs + skips the input
            }
        };
        if input.is_emlx {
            match extract_emlx_message(&bytes, &input.file_path) {
                Ok(eml) => {
                    let hash = hex::encode(Sha256::digest(&eml));
                    if !seen_apple.insert(hash) {
                        continue;
                    }
                    match parse_message(eml, &input.source_path) {
                        Ok(email) => handle(email),
                        Err(e) => eprintln!("[connector] emlx parse failed: {e}"),
                    }
                }
                Err(e) => eprintln!("[connector] {e}"),
            }
            continue;
        }
        for chunk in split_mbox(&bytes) {
            let eml = unescape_mbox_quoting(strip_mbox_envelope(&chunk));
            match parse_message(eml, &input.source_path) {
                Ok(email) => handle(email),
                Err(e) => eprintln!("[connector] mbox message parse failed: {e}"),
            }
        }
    }

    // Delete uploaded (storage-resident) inputs after processing.
    if credentials.get("localFilePath").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
        for input in &inputs {
            if !input.is_local {
                std::fs::remove_file(state.storage_root().join(&input.file_path)).ok();
            }
        }
        if let Some(single) = credentials.get("uploadedFilePath").and_then(|v| v.as_str()) {
            if !single.is_empty() {
                std::fs::remove_file(state.storage_root().join(single)).ok();
            }
        }
    }
    Ok(())
}
