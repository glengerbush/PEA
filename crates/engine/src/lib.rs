//! pea-engine library — the full PEA engine (ported from OpenArchiver): API router, job queue,
//! ingestion pipeline, blob storage, and fresh-dir provisioning. The Tauri
//! desktop app links this directly (R4: single process, no sidecars); the
//! pea-engine binary wraps it for standalone/debug serving and CLI imports.

pub mod api;
pub mod readers;
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


use state::AppState;
use std::path::Path;

/// Lowercase hex encoding — the digests we hash are always ASCII hex, so an
/// owned encoder is trivially correct and removes the `hex` dependency.
pub fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Whether the desktop shell should check for updates at launch. Reads the
/// `autoCheckUpdates` system setting, defaulting to true when the setting is
/// unset, the row is missing, or the config can't be read/parsed.
pub fn auto_check_updates(state: &AppState) -> bool {
    let Ok(conn) = state.pool.get() else {
        return true;
    };
    conn.query_row("SELECT config FROM system_settings LIMIT 1", [], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .and_then(|config| serde_json::from_str::<serde_json::Value>(&config).ok())
    .and_then(|v| v.get("autoCheckUpdates").and_then(|b| b.as_bool()))
    .unwrap_or(true)
}

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

/// Platform data dir: ~/Library/Application Support/PEA on macOS,
/// $XDG_DATA_HOME/pea (default ~/.local/share/pea) elsewhere.
/// Env override: PEA_DATA_DIR.
pub fn default_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("PEA_DATA_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::PathBuf::from(&home).join("Library/Application Support/PEA")
    } else {
        let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            format!("{}/.local/share", std::env::var("HOME").unwrap_or_default())
        });
        std::path::PathBuf::from(&base).join("pea")
    }
}

/// Builds an AppState for a data dir.
pub fn state_for_dir(data_dir: &Path, read_only: bool) -> Result<AppState, String> {
    let pool = state::open_pool(&data_dir.join("archive.db"), read_only)?;
    Ok(AppState {
        pool,
        data_dir: data_dir.to_path_buf(),
        duplicate_cache: std::sync::Arc::new(std::sync::Mutex::new(
            duplicates::DuplicateCache::default(),
        )),
        queue_nudge: std::sync::Arc::new(tokio::sync::Notify::new()),
        frontend_dir: std::env::var("FRONTEND_BUILD_DIR").ok().map(std::path::PathBuf::from),
    })
}
