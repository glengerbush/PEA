//! PEA engine (Rust) — headless CLI/server entry point for an archive.db.
//! Runs the same HTTP API as the desktop app against a chosen data dir:
//!   pea-engine --data-dir ~/.local/share/pea --port 47200 [--read-only]

use pea_engine::{ingest, provision, queue, state_for_dir};
use pea_engine::api::router;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get_arg = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let data_dir = get_arg("--data-dir")
        .map(PathBuf::from)
        .unwrap_or_else(pea_engine::default_data_dir);

    // Subcommand: `pea-engine import --data-dir D --mbox file.mbox [--name N]`
    // Runs fully synchronously (no tokio) — processors may use blocking I/O.
    if args.get(1).map(String::as_str) == Some("import") {
        let mbox = PathBuf::from(get_arg("--mbox").expect("--mbox required"));
        if let Err(e) = provision::provision(&data_dir) {
            eprintln!("[pea-engine] provisioning failed: {e}");
            std::process::exit(1);
        }
        match ingest::import_mbox(&data_dir, &mbox, get_arg("--name")) {
            Ok(stats) => {
                println!(
                    "[pea-engine] import done: source={} archived={} duplicates={} failed={}",
                    stats.source_id, stats.archived, stats.skipped_duplicates, stats.failed
                );
                return;
            }
            Err(e) => {
                eprintln!("[pea-engine] import failed: {e}");
                std::process::exit(1);
            }
        }
    }

    let port: u16 = get_arg("--port").and_then(|p| p.parse().ok()).unwrap_or(47200);
    let read_only = args.iter().any(|a| a == "--read-only");

    if !read_only {
        if let Err(e) = provision::provision(&data_dir) {
            eprintln!("[pea-engine] provisioning failed: {e}");
            std::process::exit(1);
        }
    }

    let state = state_for_dir(&data_dir, read_only).expect("failed to build state");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            if !read_only {
                let queue_state = state.clone();
                tokio::spawn(queue::start_queue(queue_state));
            }
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .expect("bind failed");
            println!("[pea-engine] listening on http://127.0.0.1:{port} (read_only={read_only})");
            axum::serve(listener, router(state)).await.unwrap();
        });
}
