// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! R4 desktop shell: the whole engine (API, job queue, ingestion, search)
//! runs INSIDE this process via the pea-engine library. The webview loads
//! pea://localhost/ and every request — page assets, fetches, images — is
//! answered by routing it through the axum router in-process. No sidecars,
//! no listening sockets, one binary.

use std::path::PathBuf;

use tauri::Manager;
use tower::util::ServiceExt;

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

/// Dev: the repo's SPA build. Release: the bundled resources/frontend copy.
fn frontend_dir(app: &tauri::AppHandle) -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages/frontend/build")
    } else {
        app.path()
            .resource_dir()
            .expect("resource dir")
            .join("resources/frontend")
    }
}

/// Routes one webview request through the in-process axum router.
async fn handle_request(
    router: axum::Router,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let request = request.map(axum::body::Body::from);
    let response = match router.oneshot(request).await {
        Ok(response) => response,
        Err(_) => {
            return http::Response::builder()
                .status(500)
                .body(b"engine error".to_vec())
                .unwrap();
        }
    };
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    let mut out = http::Response::builder().status(parts.status);
    for (name, value) in parts.headers.iter() {
        out = out.header(name, value);
    }
    out.body(bytes.to_vec()).unwrap_or_else(|_| {
        http::Response::builder()
            .status(500)
            .body(b"engine error".to_vec())
            .unwrap()
    })
}

fn main() {
    let data_dir = pea_engine::default_data_dir();
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    pea_engine::provision::provision(&data_dir).expect("provision data dir");
    let state = pea_engine::state_for_dir(&data_dir, false).expect("engine state");

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Second launch: surface the existing window instead of double-booting
            // (two engines writing one archive.db must never happen).
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.unminimize();
            }
        }))
        .register_asynchronous_uri_scheme_protocol("pea", move |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let router: axum::Router = {
                    let engine = app.state::<EngineRouter>();
                    engine.0.clone()
                };
                responder.respond(handle_request(router, request).await);
            });
        })
        .setup(move |app| {
            #[cfg(not(debug_assertions))]
            check_for_updates(app.handle().clone());

            // Finish wiring the engine now that the resource dir is resolvable.
            let mut state = state.clone();
            state.frontend_dir = Some(frontend_dir(app.handle()));
            tauri::async_runtime::spawn(pea_engine::queue::start_queue(state.clone()));
            app.manage(EngineRouter(pea_engine::api::router(state)));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// The engine's router, shared with the protocol handler via managed state.
struct EngineRouter(axum::Router);
