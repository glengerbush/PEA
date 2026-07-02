// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{Manager, RunEvent};

/// One-click updates: check GitHub Releases' latest.json on launch; if newer,
/// ask, download+install (signed), and relaunch. Disabled in dev builds.
#[cfg(not(debug_assertions))]
fn check_for_updates(app: tauri::AppHandle) {
	use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
	use tauri_plugin_updater::UpdaterExt;
	tauri::async_runtime::spawn(async move {
		let Ok(updater) = app.updater() else { return };
		let Ok(Some(update)) = updater.check().await else { return };
		let version = update.version.clone();
		let confirmed = app
			.dialog()
			.message(format!(
				"Open Archiver {version} is available (you have {}).\n\nDownload and install now? The app restarts when it's done — your archive is untouched.",
				update.current_version
			))
			.title("Update available")
			.buttons(MessageDialogButtons::OkCancelCustom(
				"Update now".into(),
				"Later".into(),
			))
			.blocking_show();
		if !confirmed {
			return;
		}
		match update.download_and_install(|_, _| {}, || {}).await {
			Ok(()) => app.restart(),
			Err(error) => eprintln!("update failed: {error}"),
		}
	});
}

/// The backend child (the single-process Node app). It supervises its own
/// Postgres and Meilisearch children and reaps them on SIGTERM — verified —
/// so the shell only has to manage this one process.
struct Backend(Mutex<Option<Child>>);

/// Same resolution as packages/backend/src/embedded.ts — the shell must agree
/// with the backend on where runtime.json lands.
fn data_dir() -> PathBuf {
	if let Ok(dir) = std::env::var("OA_DATA_DIR") {
		return PathBuf::from(dir);
	}
	if cfg!(target_os = "macos") {
		let home = std::env::var("HOME").unwrap_or_default();
		return PathBuf::from(home).join("Library/Application Support/OpenArchiver");
	}
	let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
		format!("{}/.local/share", std::env::var("HOME").unwrap_or_default())
	});
	PathBuf::from(base).join("open-archiver")
}

/// Dev: run the workspace build with the system node.
/// Release: run the bundled entry with the bundled node (packaged as resources
/// + sidecar by the release workflow).
fn spawn_backend(app: &tauri::AppHandle) -> std::io::Result<Child> {
	let mut cmd;
	if cfg!(debug_assertions) {
		// dev: system node + apps/open-archiver/dist/index.js
		let node = std::env::var("OA_NODE_BIN").unwrap_or_else(|_| "node".into());
		let entry =
			PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../open-archiver/dist/index.js");
		cmd = Command::new(node);
		cmd.arg(entry);
	} else {
		// release: everything ships with the bundle — the node runtime sits next
		// to this executable (externalBin) and the app payload under resources.
		let exe_dir = std::env::current_exe()
			.expect("current exe")
			.parent()
			.expect("exe dir")
			.to_path_buf();
		let resources = app.path().resource_dir().expect("resource dir").join("resources");
		let node = exe_dir.join(if cfg!(windows) { "node.exe" } else { "node" });
		cmd = Command::new(node);
		cmd.arg(resources.join("backend/index.js"))
			.env("OA_BUNDLED", "1")
			.env("FRONTEND_BUILD_DIR", resources.join("frontend"))
			.env("OA_MIGRATIONS_DIR", resources.join("backend/migrations"));
	}
	cmd.env("OA_EMBEDDED", "1")
		// Portable dead-man switch: the backend watches its parent pid and
		// shuts down gracefully if the shell disappears (macOS path).
		.env("OA_WATCH_PARENT", "1")
		// Never inherit a stray PORT_BACKEND / .env from the launch environment.
		.current_dir(data_dir());
	// Linux: kernel-level guarantee — if the shell dies for ANY reason
	// (including SIGKILL), the backend receives SIGTERM and runs its graceful
	// chain, reaping Postgres and Meilisearch. No orphans.
	#[cfg(target_os = "linux")]
	unsafe {
		use std::os::unix::process::CommandExt;
		cmd.pre_exec(|| {
			libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
			Ok(())
		});
	}
	std::fs::create_dir_all(data_dir()).ok();
	cmd.spawn()
}

/// Reads the app port from runtime.json once the backend has written it.
fn read_port() -> Option<u16> {
	let raw = std::fs::read_to_string(data_dir().join("runtime.json")).ok()?;
	let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
	json.get("appPort")?.as_u64().map(|p| p as u16)
}

/// Minimal HTTP GET /healthz — avoids pulling an HTTP client crate.
fn healthy(port: u16) -> bool {
	let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
		return false;
	};
	stream
		.set_read_timeout(Some(Duration::from_secs(2)))
		.ok();
	if stream
		.write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
		.is_err()
	{
		return false;
	}
	let mut buf = [0u8; 64];
	match stream.read(&mut buf) {
		Ok(n) if n > 12 => buf[..n].windows(6).any(|w| w == b" 200 O"),
		_ => false,
	}
}

/// SIGTERM the backend and give its graceful chain (queue drain, Postgres,
/// Meilisearch) time to complete; SIGKILL only as a last resort.
fn stop_backend(child: &mut Child) {
	#[cfg(unix)]
	unsafe {
		libc::kill(child.id() as i32, libc::SIGTERM);
	}
	#[cfg(not(unix))]
	{
		let _ = child.kill();
	}
	let deadline = Instant::now() + Duration::from_secs(40);
	while Instant::now() < deadline {
		match child.try_wait() {
			Ok(Some(_)) => return,
			Ok(None) => std::thread::sleep(Duration::from_millis(250)),
			Err(_) => return,
		}
	}
	let _ = child.kill();
	let _ = child.wait();
}

fn main() {
	// Stale runtime.json from a previous run would make the readiness poll
	// attach to a dead port; remove it before the backend rewrites it.
	let _ = std::fs::remove_file(data_dir().join("runtime.json"));

	tauri::Builder::default()
		.plugin(tauri_plugin_updater::Builder::new().build())
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
			// Second launch: focus the existing window instead of double-booting
			// (two backends on one data dir must never happen).
			if let Some(window) = app.get_webview_window("main") {
				let _ = window.set_focus();
				let _ = window.unminimize();
			}
		}))
		.setup(|app| {
			#[cfg(not(debug_assertions))]
			check_for_updates(app.handle().clone());

			let child = spawn_backend(app.handle())?;
			app.manage(Backend(Mutex::new(Some(child))));

			let handle = app.handle().clone();
			std::thread::spawn(move || {
				let deadline = Instant::now() + Duration::from_secs(180);
				let port = loop {
					if Instant::now() > deadline {
						eprintln!("backend did not become healthy in time");
						return;
					}
					if let Some(port) = read_port() {
						if healthy(port) {
							break port;
						}
					}
					std::thread::sleep(Duration::from_millis(400));
				};
				if let Some(window) = handle.get_webview_window("main") {
					let url = format!("http://127.0.0.1:{port}/").parse().unwrap();
					let _ = window.navigate(url);
				}
			});
			Ok(())
		})
		.build(tauri::generate_context!())
		.expect("error while building tauri application")
		.run(|app, event| {
			if let RunEvent::Exit = event {
				if let Some(backend) = app.try_state::<Backend>() {
					if let Some(mut child) = backend.0.lock().unwrap().take() {
						stop_backend(&mut child);
					}
				}
			}
		});
}
