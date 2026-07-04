use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OpenFlags;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Resolves a relative storage path against `root`, rejecting anything that
/// could escape it. Storage keys are app-generated, but some components are
/// derived from untrusted email content (Message-IDs, attachment names), so
/// this is the last line of defence against path traversal on read AND write.
/// Only plain path segments are accepted — `..`, absolute roots, and Windows
/// drive prefixes are all refused rather than silently stripped.
pub fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let mut out = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            _ => return Err(format!("unsafe storage path: {rel}")),
        }
    }
    // Belt and braces: the loop can't escape, but guard against symlink-free
    // surprises and future edits.
    if !out.starts_with(root) {
        return Err(format!("unsafe storage path: {rel}"));
    }
    Ok(out)
}

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub data_dir: PathBuf,
    /// Wakes the queue loop when a job is enqueued locally.
    pub queue_nudge: Arc<tokio::sync::Notify>,
    /// When set, the router serves this SPA build for non-API paths
    /// (express.static + index.html fallback in the Node bootstrap).
    pub frontend_dir: Option<PathBuf>,
}

impl AppState {
    pub fn storage_root(&self) -> PathBuf {
        self.data_dir.join("storage")
    }

    /// StorageService.put — plaintext write with parent dirs. At-rest
    /// encryption was removed: the keys lived beside the data with no
    /// passphrase, so it cost cycles without a real security boundary
    /// (disk-level protection is the OS's full-disk encryption).
    pub fn storage_put(&self, rel: &str, content: &[u8]) -> Result<(), String> {
        let path = resolve_within(&self.storage_root(), rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }

    /// StorageService.get — plain read.
    pub fn storage_get(&self, rel: &str) -> Result<Vec<u8>, String> {
        let path = resolve_within(&self.storage_root(), rel)?;
        std::fs::read(path).map_err(|e| e.to_string())
    }

    /// Absolute path of a storage-relative key, refusing any escape. For
    /// callers that need the path itself (downloads, quick-look temp copies).
    pub fn storage_abs(&self, rel: &str) -> Result<PathBuf, String> {
        resolve_within(&self.storage_root(), rel)
    }
}

/// Opens the same archive.db the Node engine uses. Read-only mode lets both
/// engines read concurrently while Node remains the writer (golden-diffing).
pub fn open_pool(db_path: &PathBuf, read_only: bool) -> Result<DbPool, String> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    };
    let manager = SqliteConnectionManager::file(db_path)
        .with_flags(flags)
        .with_init(move |conn| {
            if !read_only {
                conn.pragma_update(None, "journal_mode", "WAL")?;
                conn.pragma_update(None, "synchronous", "NORMAL")?;
            }
            conn.pragma_update(None, "busy_timeout", 5000)?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            // Execution-engine tuning (same rows/order returned): keep search
            // sort/group scratch in RAM and the hot index/row pages cached, and
            // memory-map the file to cut read syscalls on the per-keystroke path.
            conn.pragma_update(None, "cache_size", -32768)?; // 32 MiB page cache/conn
            conn.pragma_update(None, "temp_store", 2)?; // MEMORY
            conn.pragma_update(None, "mmap_size", 268_435_456i64)?; // 256 MiB
            Ok(())
        });
    r2d2::Pool::builder()
        .max_size(8)
        .build(manager)
        .map_err(|e| format!("failed to open archive.db: {e}"))
}
