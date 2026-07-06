//! EML reader — .eml files inside a local zip.

use crate::ingest::{parse_message, EmailObj};
use serde_json::Value;
use std::io::Read;

fn display_name(provider_config: &Value) -> String {
    if let Some(local) = provider_config.get("localFilePath").and_then(|v| v.as_str()) {
        let base = local.rsplit('/').next().unwrap_or(local);
        // Strip only the trailing ".zip", not every occurrence (so
        // "trip.zipped.zip" → "trip.zipped", not "tripped").
        return base.strip_suffix(".zip").unwrap_or(base).to_string();
    }
    format!("eml-import-{}", crate::search::now_ms())
}

pub fn eml_import_source(provider_config: &Value) -> String {
    display_name(provider_config)
}

/// testConnection — validates the local zip path.
pub fn validate(provider_config: &Value) -> Result<(), String> {
    let local = provider_config.get("localFilePath").and_then(|v| v.as_str()).unwrap_or("");
    if local.is_empty() {
        return Err("EML Zip file path not provided.".into());
    }
    if !local.to_lowercase().ends_with(".zip") {
        return Err("Provided file is not in the ZIP format.".into());
    }
    if !std::path::Path::new(local).exists() {
        return Err(format!("EML Zip file not found at path: {local}"));
    }
    Ok(())
}

pub fn for_each_email(
    provider_config: &Value,
    mut handle: impl FnMut(EmailObj),
) -> Result<(), String> {
    let local = provider_config.get("localFilePath").and_then(|v| v.as_str()).unwrap_or("");
    let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            // A corrupt/encrypted/unsupported member is skipped — but log it so an
            // email silently vanishing from the import is visible, not swallowed.
            Err(e) => {
                eprintln!("[eml] skipped unreadable zip entry #{i}: {e}");
                continue;
            }
        };
        let name = entry.name().to_string();
        if name.starts_with("__MACOSX/") || name.ends_with('/') {
            continue;
        }
        if !name.to_lowercase().ends_with(".eml") {
            continue;
        }
        let mut contents = Vec::new();
        if let Err(e) = entry.read_to_end(&mut contents) {
            eprintln!("[eml] failed to read zip entry {name}: {e}");
            continue;
        }
        // dirname(fileName), '' when at the zip root.
        let relative = match name.rfind('/') {
            Some(pos) => name[..pos].to_string(),
            None => String::new(),
        };
        match parse_message(contents, &relative, None) {
            Ok(email) => handle(email),
            Err(e) => eprintln!("[eml] failed to parse {name}: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn display_name_strips_only_trailing_zip() {
        assert_eq!(display_name(&json!({"localFilePath":"/x/trip.zipped.zip"})), "trip.zipped");
        assert_eq!(display_name(&json!({"localFilePath":"/x/Backup.zip"})), "Backup");
        assert_eq!(display_name(&json!({"localFilePath":"noslash.zip"})), "noslash");
    }

    #[test]
    fn eml_import_source_uses_display_name() {
        assert_eq!(eml_import_source(&json!({"localFilePath":"/x/My Mail.zip"})), "My Mail");
    }

    #[test]
    fn validate_rejects_bad_paths() {
        assert!(validate(&json!({})).is_err());
        assert!(validate(&json!({"localFilePath":""})).is_err());
        assert!(validate(&json!({"localFilePath":"/x/file.txt"})).is_err());
        // case-insensitive .zip is accepted at the format check; existence fails next
        assert!(validate(&json!({"localFilePath":"/nonexistent/X.ZIP"}))
            .unwrap_err()
            .contains("not found"));
    }

    #[test]
    fn for_each_email_reads_eml_entries_only() {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            let write = |w: &mut zip::ZipWriter<_>, name: &str, body: &str| {
                w.start_file(name, opts).unwrap();
                std::io::Write::write_all(w, body.as_bytes()).unwrap();
            };
            write(&mut w, "folder/one.eml", "From: a@x.com\nSubject: Hi\nMessage-ID: <e1@x>\n\nbody\n");
            write(&mut w, "two.eml", "From: c@x.com\nSubject: Two\nMessage-ID: <e2@x>\n\nbody\n");
            write(&mut w, "readme.txt", "not an eml");
            write(&mut w, "__MACOSX/skip.eml", "junk");
            w.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("pea-eml-foreach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("in.zip");
        std::fs::write(&path, &buf).unwrap();

        let mut seen: Vec<(String, String)> = Vec::new();
        for_each_email(&json!({ "localFilePath": path.to_str().unwrap() }), |email| {
            seen.push((email.subject.clone(), email.path.clone()));
        })
        .unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(seen.len(), 2, "only the two real .eml entries, txt + __MACOSX skipped");
        assert!(seen.iter().any(|(s, p)| s == "Hi" && p == "folder"));
        assert!(seen.iter().any(|(s, p)| s == "Two" && p.is_empty()));
    }
}
