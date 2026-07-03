use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OpenFlags;
use std::path::PathBuf;
use std::sync::Arc;

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub data_dir: PathBuf,
    /// STORAGE_ENCRYPTION_KEY (32 bytes) — encrypts files at rest.
    pub storage_key: Option<[u8; 32]>,
    /// ENCRYPTION_KEY — the CryptoService master key for source credentials.
    pub master_key: Option<String>,
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

    /// StorageService.put — encrypt-at-rest write with parent dirs.
    pub fn storage_put(&self, rel: &str, content: &[u8]) -> Result<(), String> {
        let path = self.storage_root().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = crate::crypto::encrypt_storage(content, &self.storage_key);
        std::fs::write(&path, data).map_err(|e| e.to_string())
    }

    /// StorageService.get — read + decrypt.
    pub fn storage_get(&self, rel: &str) -> Result<Vec<u8>, String> {
        let path = self.storage_root().join(rel);
        std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|c| crate::crypto::decrypt_storage(c, &self.storage_key))
    }
}

/// Opens the same archive.db the Node engine uses. Read-only mode lets both
/// engines read concurrently while Node remains the writer (golden-diffing).
pub fn open_pool(db_path: &PathBuf, read_only: bool) -> DbPool {
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
            Ok(())
        });
    r2d2::Pool::builder()
        .max_size(8)
        .build(manager)
        .expect("failed to open archive.db")
}
