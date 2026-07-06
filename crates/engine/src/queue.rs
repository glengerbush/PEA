//! Port of jobs/queue.ts — the in-process SQLite job queue: claim loop with
//! per-queue concurrency, exponential backoff (1s·2^n, cap 5min), singleton
//! suppression, boot recovery of interrupted jobs, and the retention sweep.

use crate::state::AppState;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

pub const QUEUE_NAMES: [&str; 3] = ["ingestion", "indexing", "remote-content"];
const POLL_MS: u64 = 500;
const BACKOFF_BASE_MS: i64 = 1000;
const BACKOFF_CAP_MS: i64 = 5 * 60 * 1000;

fn now_ms() -> i64 {
    crate::search::now_ms()
}

pub struct SendOptions<'a> {
    /// Retries after the first attempt. 0 = run exactly once. Default 4.
    pub retry_limit: i64,
    pub singleton_key: Option<&'a str>,
    pub start_after: Option<i64>,
}

impl Default for SendOptions<'_> {
    fn default() -> Self {
        Self { retry_limit: 4, singleton_key: None, start_after: None }
    }
}

pub fn master_job_options(singleton_key: &str) -> SendOptions<'_> {
    SendOptions { retry_limit: 0, singleton_key: Some(singleton_key), start_after: None }
}

/// A one-shot job: run exactly once. File imports are single-mailbox and
/// deterministic, so a failed process-mailbox won't recover on the queue's
/// backoff — and retrying is actively broken, because the first failure is
/// `is_last` and finalizes (deletes) the import session, so every retry then
/// errors with "session not found". One shot → clean terminal 'error' state.
pub fn no_retry() -> SendOptions<'static> {
    SendOptions { retry_limit: 0, singleton_key: None, start_after: None }
}

/// sendJob — returns the job id, or None when singleton-suppressed.
pub fn send_job(
    state: &AppState,
    queue: &str,
    name: &str,
    payload: &Value,
    options: SendOptions,
) -> Option<String> {
    let conn = state.pool.get().unwrap();
    if let Some(key) = options.singleton_key {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM jobs WHERE queue = ? AND singleton_key = ? \
                 AND state IN ('pending', 'active') LIMIT 1",
                rusqlite::params![queue, key],
                |r| r.get(0),
            )
            .ok();
        if existing.is_some() {
            return None;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = conn.execute(
        "INSERT INTO jobs (id, queue, name, payload, max_attempts, run_at, singleton_key) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            id,
            queue,
            name,
            payload.to_string(),
            options.retry_limit + 1,
            options.start_after.unwrap_or_else(now_ms),
            options.singleton_key,
        ],
    ) {
        // Enqueue failed (e.g. transient SQLITE_BUSY). Log rather than panic:
        // send_job runs inside worker threads, and a panic there would unwind
        // past the concurrency-slot release and permanently stall the queue.
        // `None` means "not enqueued" here (callers ignore the return value).
        eprintln!("[queue] failed to enqueue {queue}/{name}: {e}");
        return None;
    }
    state.queue_nudge.notify_one();
    Some(id)
}

/// removeJobsBySourceId — clears queued/failed ingestion jobs for a source.
pub fn remove_jobs_by_source_id(conn: &Connection, source_id: &str) -> i64 {
    conn.execute(
        "DELETE FROM jobs WHERE queue = 'ingestion' AND state IN ('pending', 'failed') \
         AND json_extract(payload, '$.ingestionSourceId') = ?",
        [source_id],
    )
    .map(|n| n as i64)
    .unwrap_or(0)
}

struct ClaimedJob {
    id: String,
    queue: String,
    name: String,
    payload: Value,
    attempts: i64,
    max_attempts: i64,
}

fn claim(conn: &Connection, queue: &str, free: i64) -> Vec<ClaimedJob> {
    if free <= 0 {
        return Vec::new();
    }
    let now = now_ms();
    let mut stmt = conn
        .prepare(
            "SELECT id, name, payload, attempts, max_attempts FROM jobs \
             WHERE queue = ? AND state = 'pending' AND run_at <= ? \
             ORDER BY created_at ASC LIMIT ?",
        )
        .unwrap();
    let candidates: Vec<ClaimedJob> = stmt
        .query_map(rusqlite::params![queue, now, free], |row| {
            Ok(ClaimedJob {
                id: row.get(0)?,
                queue: queue.to_string(),
                name: row.get(1)?,
                payload: serde_json::from_str(&row.get::<_, String>(2)?)
                    .unwrap_or(Value::Null),
                attempts: row.get(3)?,
                max_attempts: row.get(4)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    // Flip pending→active atomically and only return jobs we actually won. The
    // `AND state = 'pending'` guard + rows-affected check means a job whose claim
    // write is lost (or was already taken) is left for a later tick, never run
    // while still 'pending' — otherwise the next poll could claim and run it again.
    let mut claimed = Vec::new();
    for job in candidates {
        match conn.execute(
            "UPDATE jobs SET state = 'active', started_at = ?, attempts = ? \
             WHERE id = ? AND state = 'pending'",
            rusqlite::params![now, job.attempts + 1, job.id],
        ) {
            Ok(1) => claimed.push(job),
            _ => {}
        }
    }
    claimed
}

fn finish(conn: &Connection, job: &ClaimedJob, result: Result<(), String>) {
    // Retry the state write briefly. A silently dropped completion write would
    // leave the job 'active' and re-run it after the next restart; busy_timeout
    // already waits out most contention, so a couple of retries closes the gap.
    let durable = |sql: &str, params: &[&dyn rusqlite::ToSql]| {
        for attempt in 0..3 {
            if conn.execute(sql, params).is_ok() {
                return;
            }
            if attempt < 2 {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        eprintln!("[queue] {}:{} state write failed after retries", job.queue, job.name);
    };
    match result {
        Ok(()) => durable(
            "UPDATE jobs SET state = 'completed', finished_at = ?, error = NULL WHERE id = ?",
            rusqlite::params![now_ms(), job.id],
        ),
        Err(message) => {
            let attempts = job.attempts + 1;
            if attempts >= job.max_attempts {
                durable(
                    "UPDATE jobs SET state = 'failed', finished_at = ?, error = ? WHERE id = ?",
                    rusqlite::params![now_ms(), message, job.id],
                );
                eprintln!("[queue] {}:{} failed permanently: {message}", job.queue, job.name);
            } else {
                let delay = (BACKOFF_BASE_MS << (attempts - 1).min(20)).min(BACKOFF_CAP_MS);
                durable(
                    "UPDATE jobs SET state = 'pending', run_at = ?, error = ? WHERE id = ?",
                    rusqlite::params![now_ms() + delay, message, job.id],
                );
            }
        }
    }
}

/// Boot-time recovery: an 'active' job is a leftover from a crash. Re-queue it
/// only if it still has attempts left; a job that had already used its last
/// attempt (e.g. a no_retry process-mailbox that crashed after committing) is
/// marked 'failed' rather than re-run, which would double-count its work.
pub fn recover_interrupted(conn: &Connection) {
    conn.execute(
        "UPDATE jobs SET \
           state = CASE WHEN attempts >= max_attempts THEN 'failed' ELSE 'pending' END, \
           run_at = ?, \
           finished_at = CASE WHEN attempts >= max_attempts THEN ? ELSE finished_at END \
         WHERE state = 'active'",
        rusqlite::params![now_ms(), now_ms()],
    )
    .ok();
}

fn retention_sweep(conn: &Connection) {
    conn.execute_batch(&format!(
        "DELETE FROM jobs WHERE \
         (state = 'completed' AND finished_at < (unixepoch() * 1000) - {}) \
         OR (state = 'failed' AND finished_at < (unixepoch() * 1000) - {})",
        2 * 24 * 3600 * 1000i64,
        14 * 24 * 3600 * 1000i64
    ))
    .ok();
}

fn concurrency_for(queue: &str) -> i64 {
    match queue {
        "ingestion" => std::env::var("INGESTION_WORKER_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5),
        "indexing" => 1,
        "remote-content" => 2,
        _ => 1,
    }
}

/// The worker loop — poll + nudge driven; each claimed job runs on the
/// blocking pool through the processor dispatch in `crate::processors`.
pub async fn start_queue(state: AppState) {
    if let Ok(conn) = state.pool.get() {
        recover_interrupted(&conn);
    }
    // One-shot cleanup of stale tracker "failures" recorded before the tracking
    // filter recognized them (runs off the async executor; self-limiting).
    {
        let state = state.clone();
        tokio::task::spawn_blocking(move || crate::remote_content::sweep_tracking_assets(&state));
    }
    let running: Arc<HashMap<&'static str, AtomicI64>> = Arc::new(
        QUEUE_NAMES.iter().map(|q| (*q, AtomicI64::new(0))).collect(),
    );

    // Retention + stale-session sweep every 10 minutes. (The Node engine's
    // continuous-sync cron is gone: every provider is a one-time file import,
    // so the minutely schedule only produced no-op or unprocessable jobs.)
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
            loop {
                interval.tick().await;
                if let Ok(conn) = state.pool.get() {
                    retention_sweep(&conn);
                    crate::sessions::clean_stale_sessions(&conn);
                }
            }
        });
    }

    let mut interval = tokio::time::interval(std::time::Duration::from_millis(POLL_MS));
    loop {
        tokio::select! {
            _ = interval.tick() => {},
            _ = state.queue_nudge.notified() => {},
        }
        for queue in QUEUE_NAMES {
            let counter = &running[queue];
            let free = concurrency_for(queue) - counter.load(Ordering::SeqCst);
            if free <= 0 {
                continue; // fully busy — also avoids a `LIMIT <= 0` claim
            }
            // A transient pool-checkout timeout must NOT kill this loop: it is
            // spawned unsupervised, so an `.unwrap()` panic here would permanently
            // stop all background processing. Skip this tick and retry next poll.
            let jobs = match state.pool.get() {
                Ok(conn) => claim(&conn, queue, free),
                Err(_) => continue,
            };
            for job in jobs {
                counter.fetch_add(1, Ordering::SeqCst);
                let state = state.clone();
                let running = running.clone();
                tokio::task::spawn_blocking(move || {
                    // Catch a panicking job so it is marked failed (not left
                    // 'active') AND the concurrency slot is ALWAYS released — a
                    // panic escaping here would leak the slot and stall the queue.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::processors::dispatch(&state, &job.queue, &job.name, &job.payload)
                    }))
                    .unwrap_or_else(|_| Err(format!("job {}/{} panicked", job.queue, job.name)));
                    // Record the outcome if we can, but ALWAYS release the slot and
                    // nudge afterwards — a failed pool checkout here must not leak
                    // the slot (which would stall this queue for the whole run). A
                    // job left 'active' by a missed write is re-queued on next boot.
                    match state.pool.get() {
                        Ok(conn) => finish(&conn, &job, result),
                        Err(e) => eprintln!(
                            "[queue] could not record result for {}/{}: {e}",
                            job.queue, job.name
                        ),
                    }
                    running[job.queue.as_str()].fetch_sub(1, Ordering::SeqCst);
                    state.queue_nudge.notify_one();
                });
            }
        }
    }
}

/// Synchronous drain for the CLI import: claim and run jobs single-threaded
/// until every queue is empty of runnable work. Backoff-delayed retries are
/// honoured by sleeping to the nearest run_at.
pub fn drain_for_cli(state: &AppState) -> Result<(), String> {
    loop {
        let mut ran_any = false;
        for queue in QUEUE_NAMES {
            loop {
                let jobs = {
                    let conn = state.pool.get().map_err(|e| e.to_string())?;
                    claim(&conn, queue, 1)
                };
                let Some(job) = jobs.into_iter().next() else { break };
                let result = crate::processors::dispatch(state, &job.queue, &job.name, &job.payload);
                let conn = state.pool.get().map_err(|e| e.to_string())?;
                finish(&conn, &job, result);
                ran_any = true;
            }
        }
        if ran_any {
            continue;
        }
        // Nothing claimable now — check for future-scheduled pending work.
        let next_run: Option<i64> = {
            let conn = state.pool.get().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT min(run_at) FROM jobs WHERE state = 'pending'",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten()
        };
        match next_run {
            Some(at) => {
                let wait = (at - now_ms()).max(0) as u64;
                std::thread::sleep(std::time::Duration::from_millis(wait.min(5000) + 10));
            }
            None => return Ok(()),
        }
    }
}
