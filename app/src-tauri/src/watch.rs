//! Keeps the open pack's lists honest: when files in its mods, resource packs or shaders folders change behind
//! Obelisk's back (dropped in by hand, removed by another program), the window is told to read them again.

use notify::{RecursiveMode, Watcher};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::Emitter;

static CURRENT: Mutex<Option<(String, notify::RecommendedWatcher)>> = Mutex::new(None);

/// Watch this pack's content folders (replacing the previous pack's watch). An empty id stops watching.
#[tauri::command]
pub async fn watch_content(app: tauri::AppHandle, instance_id: String) -> Result<(), String> {
    let mut cur = CURRENT.lock().unwrap();
    if cur.as_ref().is_some_and(|(id, _)| *id == instance_id) {
        return Ok(());
    }
    *cur = None; // dropping the old watcher ends its thread
    if instance_id.is_empty() {
        return Ok(());
    }
    let game = super::game(&instance_id)?;
    let (tx, rx) = mpsc::channel::<()>();
    let mut w = notify::recommended_watcher(move |r: notify::Result<notify::Event>| {
        if r.is_ok_and(|e| !e.kind.is_access()) {
            let _ = tx.send(());
        }
    }).map_err(|e| e.to_string())?;
    for kind in ["mods", "resourcepacks", "shaderpacks"] {
        let dir = game.join(kind);
        if dir.is_dir() {
            w.watch(&dir, RecursiveMode::NonRecursive).map_err(|e| e.to_string())?;
        }
    }
    let id = instance_id.clone();
    std::thread::spawn(move || {
        // One refresh per burst of changes (an update swaps many files at once).
        while rx.recv().is_ok() {
            while rx.recv_timeout(Duration::from_millis(500)).is_ok() {}
            let _ = app.emit("content-changed", &id);
        }
    });
    *cur = Some((instance_id, w));
    Ok(())
}
