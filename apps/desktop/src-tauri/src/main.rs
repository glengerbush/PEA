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
                "PEA {version} is available (you have {}).\n\nDownload and install now? The app restarts when it's done — your archive is untouched.",
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

/// Native file/folder picker, exposed to the SPA as an HTTP endpoint — the
/// frontend has no Tauri IPC (everything flows over pea://), so the import
/// form fetches /api/v1/native/pick-{file,folder} and gets the chosen path.
async fn handle_native_pick(app: tauri::AppHandle, mode: String) -> http::Response<Vec<u8>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, mut rx) = tauri::async_runtime::channel::<Option<tauri_plugin_dialog::FilePath>>(1);
    let dialog = app.dialog().file();
    if mode == "folder" {
        dialog.pick_folder(move |path| {
            let _ = tx.try_send(path);
        });
    } else {
        dialog.pick_file(move |path| {
            let _ = tx.try_send(path);
        });
    }
    let picked = rx
        .recv()
        .await
        .flatten()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned());
    let body = match picked {
        Some(path) => format!("{{\"path\":{}}}", serde_json::Value::String(path)),
        None => "{\"path\":null}".to_string(),
    };
    http::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(body.into_bytes())
        .unwrap()
}

/// Native clipboard write, exposed as POST /api/v1/native/clipboard with a
/// {text, html?} JSON body. The WebKitGTK webview rejects the async Clipboard
/// API outside a strict user-gesture window, so the shell writes both flavors
/// via arboard instead. The clipboard instance is kept alive for the app's
/// lifetime — on X11/Wayland the selection is only served while it exists.
async fn handle_native_clipboard(body: Vec<u8>) -> http::Response<Vec<u8>> {
    let respond = |status: u16, message: &str| {
        http::Response::builder()
            .status(status)
            .header("content-type", "text/plain")
            .body(message.as_bytes().to_vec())
            .unwrap()
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return respond(400, "Invalid JSON body");
    };
    let Some(text) = parsed.get("text").and_then(|v| v.as_str()).map(String::from) else {
        return respond(400, "Missing text");
    };
    let html = parsed.get("html").and_then(|v| v.as_str()).map(String::from);

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        static CLIPBOARD: std::sync::Mutex<Option<arboard::Clipboard>> =
            std::sync::Mutex::new(None);
        let mut guard = CLIPBOARD.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            *guard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        let clipboard = guard.as_mut().unwrap();
        match html {
            Some(html) => clipboard.set_html(html, Some(text)),
            None => clipboard.set_text(text),
        }
        .map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(())) => respond(204, ""),
        Ok(Err(error)) => respond(500, &format!("Clipboard write failed: {error}")),
        Err(_) => respond(500, "Clipboard write failed"),
    }
}

/// A JSON HTTP response for the native endpoints below.
fn json_response(status: u16, body: &str) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body.as_bytes().to_vec())
        .unwrap()
}

/// Manual update check via Tauri's signed release updater, exposed as
/// GET /api/v1/native/update-check. Unlike the engine's commit-based check,
/// this reflects the actual installable GitHub release — the mechanism that can
/// self-install — and drives the Settings "Install & restart" button. The
/// `error` field is populated when the updater is unavailable (e.g. outside the
/// packaged app) or the network request fails.
async fn handle_native_update_check(app: tauri::AppHandle) -> http::Response<Vec<u8>> {
    use tauri_plugin_updater::UpdaterExt;
    let repo = std::env::var("PEA_UPDATE_REPO").unwrap_or_else(|_| "glengerbush/PEA".into());
    let releases_url = format!("https://github.com/{repo}/releases");
    let current = app.package_info().version.to_string();
    let payload = match app.updater() {
        Ok(updater) => match updater.check().await {
            Ok(Some(update)) => serde_json::json!({
                "available": true,
                "currentVersion": update.current_version,
                "version": update.version,
                "notes": update.body,
                "releasesUrl": releases_url,
            }),
            Ok(None) => serde_json::json!({
                "available": false,
                "currentVersion": current,
                "releasesUrl": releases_url,
            }),
            Err(error) => serde_json::json!({
                "available": false,
                "currentVersion": current,
                "error": error.to_string(),
                "releasesUrl": releases_url,
            }),
        },
        Err(error) => serde_json::json!({
            "available": false,
            "currentVersion": current,
            "error": error.to_string(),
            "releasesUrl": releases_url,
        }),
    };
    json_response(200, &payload.to_string())
}

/// Download + install the available signed release and relaunch, exposed as
/// POST /api/v1/native/update-install. The install runs in the background so
/// the HTTP response returns immediately; the app restarts once it finishes.
async fn handle_native_update_install(app: tauri::AppHandle) -> http::Response<Vec<u8>> {
    use tauri_plugin_updater::UpdaterExt;
    tauri::async_runtime::spawn(async move {
        let Ok(updater) = app.updater() else { return };
        let Ok(Some(update)) = updater.check().await else { return };
        match update.download_and_install(|_, _| {}, || {}).await {
            Ok(()) => app.restart(),
            Err(error) => eprintln!("update failed: {error}"),
        }
    });
    json_response(202, "{\"started\":true}")
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
                if let Some(mode) = request
                    .uri()
                    .path()
                    .strip_prefix("/api/v1/native/pick-")
                    .map(String::from)
                {
                    responder.respond(handle_native_pick(app, mode).await);
                    return;
                }
                if request.uri().path() == "/api/v1/native/clipboard" {
                    responder.respond(handle_native_clipboard(request.into_body()).await);
                    return;
                }
                if request.uri().path() == "/api/v1/native/update-check" {
                    responder.respond(handle_native_update_check(app).await);
                    return;
                }
                if request.uri().path() == "/api/v1/native/update-install" {
                    responder.respond(handle_native_update_install(app).await);
                    return;
                }
                let router: axum::Router = {
                    let engine = app.state::<EngineRouter>();
                    engine.0.clone()
                };
                responder.respond(handle_request(router, request).await);
            });
        })
        .on_page_load(|_webview, _payload| {
            // Dev convenience: bind Ctrl/Cmd+R and F5 to reload the webview.
            // Re-injected on every page load so it survives reloads. The whole
            // block is compiled out of release builds.
            #[cfg(debug_assertions)]
            let _ = _webview.eval(
                "if(!window.__peaDevReload){window.__peaDevReload=1;\
                 document.addEventListener('keydown',function(e){\
                 if(((e.ctrlKey||e.metaKey)&&(e.key==='r'||e.key==='R'))||e.key==='F5'){\
                 e.preventDefault();location.reload();}});}",
            );
        })
        .setup(move |app| {
            // Launch check only when "Automatically check for updates" is on
            // (default). Off → the user drives it from Settings. Dev builds never
            // auto-check. Either way the install still asks first.
            #[cfg(not(debug_assertions))]
            if pea_engine::auto_check_updates(&state) {
                check_for_updates(app.handle().clone());
            }

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
