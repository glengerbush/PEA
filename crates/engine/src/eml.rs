//! Port of EMLConnector — .eml files inside an uploaded (or local) zip.

use crate::ingest::{parse_message, EmailObj};
use crate::state::AppState;
use serde_json::Value;
use std::io::Read;

fn display_name(credentials: &Value) -> String {
    if let Some(name) = credentials.get("uploadedFileName").and_then(|v| v.as_str()) {
        return name.to_string();
    }
    if let Some(local) = credentials.get("localFilePath").and_then(|v| v.as_str()) {
        let base = local.rsplit('/').next().unwrap_or(local);
        return base.replace(".zip", "");
    }
    format!("eml-import-{}", crate::search::now_ms())
}

pub fn eml_user_email(credentials: &Value) -> String {
    format!("{}@eml.local", display_name(credentials).replace(' ', ".").to_lowercase())
}

/// testConnection — same validation messages as the Node connector.
pub fn validate(state: &AppState, credentials: &Value) -> Result<(), String> {
    let local = credentials.get("localFilePath").and_then(|v| v.as_str()).unwrap_or("");
    let uploaded = credentials.get("uploadedFilePath").and_then(|v| v.as_str()).unwrap_or("");
    let file_path = if !local.is_empty() { local } else { uploaded };
    if file_path.is_empty() {
        return Err("EML Zip file path not provided.".into());
    }
    if !file_path.contains(".zip") {
        return Err("Provided file is not in the ZIP format.".into());
    }
    let exists = if !local.is_empty() {
        std::path::Path::new(local).exists()
    } else {
        state.storage_root().join(uploaded).is_file()
    };
    if !exists {
        if !local.is_empty() {
            return Err(format!("EML Zip file not found at path: {local}"));
        }
        return Err(
            "Uploaded EML Zip file not found. The upload may not have finished yet, or it failed."
                .into(),
        );
    }
    Ok(())
}

pub fn for_each_email(
    state: &AppState,
    credentials: &Value,
    mut handle: impl FnMut(EmailObj),
) -> Result<(), String> {
    let local = credentials.get("localFilePath").and_then(|v| v.as_str()).unwrap_or("");
    let uploaded = credentials.get("uploadedFilePath").and_then(|v| v.as_str()).unwrap_or("");
    let bytes = if !local.is_empty() {
        std::fs::read(local).map_err(|e| e.to_string())?
    } else {
        state.storage_get(uploaded)?
    };
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        if name.starts_with("__MACOSX/") || name.ends_with('/') {
            continue;
        }
        if !name.to_lowercase().ends_with(".eml") {
            continue;
        }
        let mut contents = Vec::new();
        if entry.read_to_end(&mut contents).is_err() {
            continue;
        }
        // dirname(fileName), '' when at the zip root.
        let relative = match name.rfind('/') {
            Some(pos) => name[..pos].to_string(),
            None => String::new(),
        };
        match parse_message(contents, &relative) {
            Ok(email) => handle(email),
            Err(e) => eprintln!("[eml] failed to parse {name}: {e}"),
        }
    }
    // Delete the uploaded zip after processing (Node's finally block).
    if local.is_empty() && !uploaded.is_empty() {
        std::fs::remove_file(state.storage_root().join(uploaded)).ok();
    }
    Ok(())
}
