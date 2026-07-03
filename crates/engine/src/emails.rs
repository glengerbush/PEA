//! Ports of ArchivedEmailService.deleteArchivedEmail, ArchiveTagService, and
//! ContactsService (csv/vcf import).

use crate::state::AppState;
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Value};

/// deleteArchivedEmail — attachments (ref-counted), storage, FTS, row, and
/// auto-removal of an emptied file-based source in a terminal state.
pub fn delete_archived_email(state: &AppState, conn: &Connection, email_id: &str) -> Result<(), String> {
    let email: (String, bool, Option<String>) = conn
        .query_row(
            "SELECT storage_path, has_attachments, ingestion_source_id FROM archived_emails WHERE id = ?",
            [email_id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?)),
        )
        .map_err(|_| "Archived email not found".to_string())?;
    let (storage_path, has_attachments, source_id) = email;

    if has_attachments {
        let mut stmt = conn
            .prepare(
                "SELECT a.id, a.storage_path FROM email_attachments ea \
                 INNER JOIN attachments a ON ea.attachment_id = a.id WHERE ea.email_id = ?",
            )
            .map_err(|e| e.to_string())?;
        let attachments: Vec<(String, String)> = stmt
            .query_map([email_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        for (attachment_id, att_storage_path) in attachments {
            conn.execute(
                "DELETE FROM email_attachments WHERE email_id = ? AND attachment_id = ?",
                rusqlite::params![email_id, attachment_id],
            )
            .map_err(|e| e.to_string())?;
            let refs: i64 = conn
                .query_row(
                    "SELECT count(*) FROM email_attachments WHERE attachment_id = ?",
                    [&attachment_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if refs == 0 {
                std::fs::remove_file(state.storage_root().join(&att_storage_path)).ok();
                conn.execute("DELETE FROM attachments WHERE id = ?", [&attachment_id])
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    std::fs::remove_file(state.storage_root().join(&storage_path)).ok();
    conn.execute(
        "DELETE FROM email_fts WHERE rowid = (SELECT rowid FROM archived_emails WHERE id = ?)",
        [email_id],
    )
    .ok();
    conn.execute("DELETE FROM archived_emails WHERE id = ?", [email_id])
        .map_err(|e| e.to_string())?;

    // Auto-remove an emptied, finished, file-based source (best-effort).
    if let Some(source_id) = source_id {
        let remaining: i64 = conn
            .query_row(
                "SELECT count(*) FROM archived_emails WHERE ingestion_source_id = ?",
                [&source_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if remaining == 0 {
            let source: Option<(String, String)> = conn
                .query_row(
                    "SELECT provider, status FROM ingestion_sources WHERE id = ?",
                    [&source_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            if let Some((provider, status)) = source {
                let terminal = status == "imported" || status == "error";
                if crate::sources::FILE_BASED_PROVIDERS.contains(&provider.as_str()) && terminal {
                    crate::sources::delete_source(state, conn, &source_id).ok();
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ArchiveTagService
// ---------------------------------------------------------------------------

const MAX_BULK_TAG_SIZE: usize = 1000;
const MAX_TAGS_PER_EMAIL: usize = 64;
const MAX_TAG_LENGTH: usize = 64;

fn normalize_tag(raw: &str) -> String {
    static CTRL: Lazy<Regex> = Lazy::new(|| Regex::new("[\u{0000}-\u{001F}\u{007F}]").unwrap());
    static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    static HASHES: Lazy<Regex> = Lazy::new(|| Regex::new(r"^#+").unwrap());
    let v = CTRL.replace_all(raw, "");
    let v = WS.replace_all(&v, " ");
    let v = v.trim();
    let v = HASHES.replace(v, "");
    let v = v.trim();
    v.chars().take(MAX_TAG_LENGTH).collect()
}

fn normalize_tags(raw: &Value) -> Vec<String> {
    let source: Vec<String> = match raw {
        Value::String(s) => s.split(',').map(String::from).collect(),
        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    };
    let mut tags = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw_tag in source {
        let tag = normalize_tag(&raw_tag);
        let key = tag.to_lowercase();
        if tag.is_empty() || seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        tags.push(tag);
    }
    tags
}

fn apply_tag_changes(current: &Value, add: &[String], remove: &[String]) -> Vec<String> {
    let remove_keys: std::collections::HashSet<String> =
        remove.iter().map(|t| t.to_lowercase()).collect();
    let mut next = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tag in normalize_tags(current) {
        let key = tag.to_lowercase();
        if remove_keys.contains(&key) || seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        next.push(tag);
    }
    for tag in add {
        let key = tag.to_lowercase();
        if remove_keys.contains(&key) || seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        next.push(tag.clone());
        if next.len() >= MAX_TAGS_PER_EMAIL {
            break;
        }
    }
    next
}

/// updateEmailTags — returns the Node response body or an error message
/// (the endpoint maps errors to 400 {message}).
pub fn update_email_tags(conn: &Connection, dto: &Value) -> Result<Value, String> {
    let mut email_ids: Vec<String> = Vec::new();
    for v in dto.get("emailIds").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
        if let Some(s) = v.as_str() {
            if !s.is_empty() && !email_ids.contains(&s.to_string()) {
                email_ids.push(s.to_string());
            }
        }
    }
    if email_ids.is_empty() {
        return Err("At least one email must be selected".into());
    }
    if email_ids.len() > MAX_BULK_TAG_SIZE {
        return Err(format!("At most {MAX_BULK_TAG_SIZE} emails can be updated at once"));
    }
    let added = normalize_tags(dto.get("addTags").unwrap_or(&Value::Null));
    let removed = normalize_tags(dto.get("removeTags").unwrap_or(&Value::Null));
    if added.is_empty() && removed.is_empty() {
        return Err("At least one tag must be added or removed".into());
    }

    let placeholders = vec!["?"; email_ids.len()].join(", ");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, tags FROM archived_emails WHERE id IN ({placeholders})"
        ))
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map(rusqlite::params_from_iter(email_ids.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut updates: Vec<(String, Vec<String>)> = Vec::new();
    for (id, tags_raw) in rows {
        let current: Value = tags_raw
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let current_tags = normalize_tags(&current);
        let next = apply_tag_changes(&current, &added, &removed);
        if current_tags == next {
            continue;
        }
        conn.execute(
            "UPDATE archived_emails SET tags = ? WHERE id = ?",
            rusqlite::params![serde_json::to_string(&next).unwrap(), id],
        )
        .map_err(|e| e.to_string())?;
        updates.push((id, next));
    }

    // SearchService.updateDocuments — recompute the meta column per email.
    for (id, _) in &updates {
        let row: Option<(String, Option<String>, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT user_email, source_path, source_labels, tags FROM archived_emails WHERE id = ?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        let Some((user_email, source_path, labels_raw, tags_raw)) = row else { continue };
        let labels: Vec<String> = labels_raw
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let tags: Vec<String> = tags_raw
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let mut meta_parts = vec![user_email, source_path.unwrap_or_default()];
        meta_parts.extend(labels);
        meta_parts.extend(tags);
        conn.execute(
            "UPDATE email_fts SET meta = ? WHERE rowid = (SELECT rowid FROM archived_emails WHERE id = ?)",
            rusqlite::params![meta_parts.join(" "), id],
        )
        .ok();
    }

    Ok(json!({
        "requestedCount": email_ids.len(),
        "updatedCount": updates.len(),
        "addedTags": added,
        "removedTags": removed,
        "emails": updates
            .iter()
            .map(|(id, tags)| json!({ "id": id, "tags": tags }))
            .collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------------------
// ContactsService import (csv / vcf)
// ---------------------------------------------------------------------------

static EMAIL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").unwrap());

fn normalize_email(value: &str) -> String {
    value.trim().to_lowercase().trim_start_matches("mailto:").to_string()
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if in_quotes {
            if ch == '"' {
                if chars.get(i + 1) == Some(&'"') {
                    cur.push('"');
                    i += 1;
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == ',' {
            out.push(cur.trim().to_string());
            cur = String::new();
        } else {
            cur.push(ch);
        }
        i += 1;
    }
    out.push(cur.trim().to_string());
    out
}

fn parse_csv(content: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = content.split(['\n']).map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let header: Vec<String> = split_csv_line(lines[0]).iter().map(|h| h.to_lowercase()).collect();
    let find_col = |needles: &[&str]| -> Option<usize> {
        header
            .iter()
            .position(|h| needles.iter().any(|n| h == n || h.contains(n)))
    };
    let email_idx = find_col(&["e-mail address", "email address", "email", "e-mail"]);
    let name_idx = find_col(&["display name", "full name", "name"]);
    let first_idx = find_col(&["first name", "given name", "first"]);
    let last_idx = find_col(&["last name", "family name", "surname", "last"]);

    let mut results = Vec::new();
    for line in &lines[1..] {
        let cells = split_csv_line(line);
        let mut email = email_idx.and_then(|i| cells.get(i).cloned()).unwrap_or_default();
        if email.is_empty() || !EMAIL_RE.is_match(&normalize_email(&email)) {
            email = cells
                .iter()
                .find(|c| EMAIL_RE.is_match(&normalize_email(c)))
                .cloned()
                .unwrap_or_default();
        }
        let email = normalize_email(&email);
        if !EMAIL_RE.is_match(&email) {
            continue;
        }
        let mut name = name_idx
            .and_then(|i| cells.get(i))
            .map(|c| c.trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            let parts: Vec<String> = [first_idx, last_idx]
                .iter()
                .filter_map(|idx| idx.and_then(|i| cells.get(i)))
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            name = parts.join(" ");
        }
        let display = if name.is_empty() { email.clone() } else { name };
        results.push((email, display));
    }
    results
}

fn parse_vcf(content: &str) -> Vec<(String, String)> {
    static FN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^FN[:;]").unwrap());
    static N_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^N[:;]").unwrap());
    static EMAIL_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^EMAIL[:;]").unwrap());
    static PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^[A-Z]+[^:]*:").unwrap());
    let mut results = Vec::new();
    let blocks: Vec<&str> = Regex::new(r"(?i)BEGIN:VCARD")
        .unwrap()
        .split(content)
        .skip(1)
        .collect();
    for block in blocks {
        let mut fn_name = String::new();
        let mut n_name = String::new();
        let mut emails: Vec<String> = Vec::new();
        for raw in block.split(['\n']) {
            let line = raw.trim_end_matches('\r').trim();
            if FN_RE.is_match(line) {
                fn_name = PREFIX.replace(line, "").trim().to_string();
            } else if N_RE.is_match(line) && !line.to_lowercase().starts_with("note") {
                let val = PREFIX.replace(line, "").to_string();
                let mut parts = val.split(';');
                let last = parts.next().unwrap_or("").trim();
                let first = parts.next().unwrap_or("").trim();
                n_name = [first, last]
                    .iter()
                    .filter(|p| !p.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
            } else if EMAIL_LINE.is_match(line) {
                let val = normalize_email(&PREFIX.replace(line, ""));
                if EMAIL_RE.is_match(&val) {
                    emails.push(val);
                }
            }
        }
        let name = if !fn_name.is_empty() { fn_name } else { n_name };
        for email in emails {
            let display = if name.is_empty() { email.clone() } else { name.clone() };
            results.push((email, display));
        }
    }
    results
}

/// importContacts — parses, de-dupes by email (last wins), upserts.
pub fn import_contacts(conn: &Connection, format: &str, content: &str) -> Result<Value, String> {
    let parsed_raw = if format == "vcf" { parse_vcf(content) } else { parse_csv(content) };
    let mut by_email: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    for (email, name) in &parsed_raw {
        by_email.insert(email.clone(), name.clone());
    }
    let mut imported = 0usize;
    let mut updated = 0usize;
    for (email, display_name) in &by_email {
        let existing: Option<String> = conn
            .query_row("SELECT id FROM contacts WHERE email = ?", [email], |r| r.get(0))
            .ok();
        match existing {
            Some(id) => {
                conn.execute(
                    "UPDATE contacts SET display_name = ?, source = ?, updated_at = ? WHERE id = ?",
                    rusqlite::params![display_name, format, crate::search::now_ms(), id],
                )
                .map_err(|e| e.to_string())?;
                updated += 1;
            }
            None => {
                conn.execute(
                    "INSERT INTO contacts (id, email, display_name, source) VALUES (?, ?, ?, ?)",
                    rusqlite::params![uuid::Uuid::new_v4().to_string(), email, display_name, format],
                )
                .map_err(|e| e.to_string())?;
                imported += 1;
            }
        }
    }
    Ok(json!({
        "parsed": parsed_raw.len(),
        "imported": imported,
        "updated": updated,
        "skipped": parsed_raw.len() - by_email.len(),
    }))
}
