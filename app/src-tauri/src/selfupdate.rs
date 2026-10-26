//! Updates of Obelisk itself, from its signed GitHub Releases (the Tauri updater; the endpoint and public key are in
//! tauri.conf.json, the signing key never leaves the release PC). Like Discord: a start opens a small window that
//! checks for an update and installs it before Obelisk opens (Obelisk then starts again by itself, and shows what
//! changed once). While Obelisk runs, a check every 30 minutes shows the Update button ("self-update" event).

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// When this copy was built (seconds since 1970), set by build.rs.
pub const BUILD: &str = env!("OBELISK_BUILD");

/// The update window's label (its page is the "updater" route).
pub const WINDOW: &str = "updater";

/// What the new version changed, left for it by the old one (shown once, then removed).
const WHATS_NEW: &str = "whats-new.json";

/// An update is downloading or installing: the start-up window must not give way to Obelisk meanwhile.
static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Available {
    version: String,
    notes: String,
}

impl From<&Update> for Available {
    fn from(u: &Update) -> Self {
        Available { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() }
    }
}

#[derive(serde::Serialize, Clone)]
struct Progress {
    done: u64,
    total: u64,
}

async fn find(app: &tauri::AppHandle) -> Result<Option<Update>, String> {
    app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| format!("couldn't check for updates: {e}"))
}

/// A newer release, if there is one (none when offline).
#[tauri::command]
pub async fn self_update_check(app: tauri::AppHandle) -> Option<Available> {
    find(&app).await.ok().flatten().map(|u| Available::from(&u))
}

/// Download the newer release, check its signature, and install it; Obelisk closes and the new one starts.
#[tauri::command]
pub async fn self_update_install(app: tauri::AppHandle) -> Result<(), String> {
    let u = find(&app).await?.ok_or("Obelisk is up to date")?;
    install(&app, u).await
}

async fn install(app: &tauri::AppHandle, u: Update) -> Result<(), String> {
    INSTALLING.store(true, Ordering::SeqCst);
    let result = install_inner(app, u).await;
    INSTALLING.store(false, Ordering::SeqCst);
    result
}

async fn install_inner(app: &tauri::AppHandle, u: Update) -> Result<(), String> {
    // The signature is checked against tauri.conf.json's public key before anything runs.
    let mut done = 0u64;
    let bytes = u.download(|n, total| {
        done += n as u64;
        let _ = app.emit("self-update-progress", Progress { done, total: total.unwrap_or(0) });
    }, || {}).await.map_err(|e| format!("couldn't download the update: {e}"))?;
    let notes = serde_json::to_vec(&Available::from(&u)).unwrap_or_default();
    let _ = std::fs::write(crate::datadir::root().join(WHATS_NEW), notes);
    super::control::stop();
    // On Windows this starts the installer quietly (it starts the new Obelisk when done) and ends this process.
    u.install(bytes).map_err(|e| {
        let _ = std::fs::remove_file(crate::datadir::root().join(WHATS_NEW));
        format!("couldn't install the update: {e}")
    })?;
    app.restart();
}

/// The start-up window: install a waiting update (Obelisk starts again by itself), or report that there is none
/// (false: open Obelisk). A slow or absent connection doesn't hold Obelisk up for long.
#[tauri::command]
pub async fn startup_update(app: tauri::AppHandle) -> bool {
    let Ok(Ok(Some(u))) = tokio::time::timeout(std::time::Duration::from_secs(6), find(&app)).await else {
        return false;
    };
    let _ = app.emit("self-update-found", Available::from(&u));
    install(&app, u).await.is_ok()
}

/// The start-up window is done: Obelisk's own window takes its place.
#[tauri::command]
pub fn startup_done(app: tauri::AppHandle) {
    open_main(&app);
}

fn open_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.close();
    }
}

/// What this version changed, once, right after an update installed it.
#[tauri::command]
pub fn whats_new(app: tauri::AppHandle) -> Option<Available> {
    let path = crate::datadir::root().join(WHATS_NEW);
    let note: Available = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let _ = std::fs::remove_file(&path);
    (note.version == app.package_info().version.to_string()).then_some(note)
}

/// A normal start: the small update window comes first and Obelisk's own window (created hidden) after it. If that
/// window can't be made, or its page never answers, Obelisk opens anyway.
pub fn start_window(app: &tauri::AppHandle) {
    let made = tauri::WebviewWindowBuilder::new(app, WINDOW, tauri::WebviewUrl::App("updater".into()))
        .title("Obelisk")
        .inner_size(300.0, 340.0)
        .resizable(false)
        .maximizable(false)
        .decorations(false)
        .shadow(true)
        .center()
        .background_color(tauri::window::Color(0x16, 0x17, 0x1a, 0xff))
        .build();
    if made.is_err() {
        return open_main(app);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        if !INSTALLING.load(Ordering::SeqCst) && app.get_webview_window(WINDOW).is_some() {
            open_main(&app);
        }
    });
}

/// Every 30 minutes while Obelisk runs: offer a new release with the Update button. Started with Windows (no
/// start-up window), a waiting update installs straight away unless something would be cut off (`idle` false).
pub fn watch(app: tauri::AppHandle, at_start: bool, idle: impl Fn() -> bool + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        if at_start {
            if let Ok(Some(u)) = find(&app).await {
                if idle() {
                    let _ = install(&app, u).await;
                } else {
                    let _ = app.emit("self-update", Available::from(&u));
                }
            }
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
            if let Ok(Some(u)) = find(&app).await {
                let _ = app.emit("self-update", Available::from(&u));
            }
        }
    });
}
