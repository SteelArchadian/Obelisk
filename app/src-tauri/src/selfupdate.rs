//! Updates of Obelisk itself, from its signed GitHub Releases (the Tauri updater; the endpoint and public key are in
//! tauri.conf.json, the signing key never leaves the release PC). A fresh start installs a waiting update by itself;
//! while Obelisk runs, a check every 30 minutes shows the Update button ("self-update" event).

use tauri::Emitter;
use tauri_plugin_updater::{Update, UpdaterExt};

/// When this copy was built (seconds since 1970), set by build.rs.
pub const BUILD: &str = env!("OBELISK_BUILD");

#[derive(serde::Serialize, Clone)]
pub struct Available {
    version: String,
    notes: String,
}

impl From<&Update> for Available {
    fn from(u: &Update) -> Self {
        Available { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() }
    }
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
    // The signature is checked against tauri.conf.json's public key before anything runs.
    let bytes = u.download(|_, _| {}, || {}).await.map_err(|e| format!("couldn't download the update: {e}"))?;
    super::control::stop();
    // On Windows this starts the installer (which starts the new Obelisk when done) and ends this process.
    u.install(bytes).map_err(|e| format!("couldn't install the update: {e}"))?;
    app.restart();
}

/// At start: a waiting update installs straight away, unless something would be cut off (`idle` false).
/// Afterwards: checked every 30 minutes and offered with the Update button.
pub fn watch(app: tauri::AppHandle, idle: impl Fn() -> bool + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        if let Ok(Some(u)) = find(&app).await {
            if idle() {
                let _ = install(&app, u).await;
            } else {
                let _ = app.emit("self-update", Available::from(&u));
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
