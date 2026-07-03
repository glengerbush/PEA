//! pea-engine library — the full OpenArchiver engine: API router, job queue,
//! ingestion pipeline, storage crypto, and fresh-dir provisioning. The Tauri
//! desktop app links this directly (R4: single process, no sidecars); the
//! pea-engine binary wraps it for standalone/debug serving and CLI imports.

pub mod api;
pub mod connectors;
pub mod crypto;
pub mod duplicates;
pub mod emails;
pub mod eml;
pub mod handlers;
pub mod ingest;
pub mod preview;
pub mod processors;
pub mod provision;
pub mod queue;
pub mod remote_content;
pub mod search;
pub mod sessions;
pub mod sources;
pub mod state;
pub mod writes;


use serde_json::{json, Value};
use state::AppState;
use std::path::Path;

/// ISO-8601 with milliseconds — matches JS Date.toJSON() for epoch-ms ints.
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    // civil-from-days (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

/// Default data dir for PEA, with a one-time automatic migration from the
/// pre-rename Open Archiver location (same-filesystem rename — atomic).
/// Env override: PEA_DATA_DIR (or the legacy OA_DATA_DIR).
pub fn default_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("PEA_DATA_DIR").or_else(|_| std::env::var("OA_DATA_DIR")) {
        return std::path::PathBuf::from(dir);
    }
    let (new_dir, old_dir) = if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").unwrap_or_default();
        (
            std::path::PathBuf::from(&home).join("Library/Application Support/PEA"),
            std::path::PathBuf::from(&home).join("Library/Application Support/OpenArchiver"),
        )
    } else {
        let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            format!("{}/.local/share", std::env::var("HOME").unwrap_or_default())
        });
        (
            std::path::PathBuf::from(&base).join("pea"),
            std::path::PathBuf::from(&base).join("open-archiver"),
        )
    };
    if !new_dir.exists() && old_dir.exists() {
        if std::fs::rename(&old_dir, &new_dir).is_err() {
            // Migration failed (permissions / cross-device) — keep the old home.
            return old_dir;
        }
        eprintln!(
            "[pea] migrated archive dir {} -> {}",
            old_dir.display(),
            new_dir.display()
        );
    }
    new_dir
}

/// Builds an AppState for a data dir: pool + keys from secrets.json.
pub fn state_for_dir(data_dir: &Path, read_only: bool) -> Result<AppState, String> {
    let secrets: Value = std::fs::read_to_string(data_dir.join("secrets.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let storage_key = secrets
        .get("storageEncryptionKey")
        .and_then(|k| k.as_str())
        .and_then(|hexkey| {
            let bytes = hex::decode(hexkey).ok()?;
            <[u8; 32]>::try_from(bytes.as_slice()).ok()
        });
    let master_key = secrets
        .get("encryptionKey")
        .and_then(|k| k.as_str())
        .map(String::from);
    let pool = state::open_pool(&data_dir.join("archive.db"), read_only);
    Ok(AppState {
        pool,
        data_dir: data_dir.to_path_buf(),
        storage_key,
        master_key,
        queue_nudge: std::sync::Arc::new(tokio::sync::Notify::new()),
        frontend_dir: std::env::var("FRONTEND_BUILD_DIR").ok().map(std::path::PathBuf::from),
    })
}

