//! Obelisk's window comes back exactly where it was (app-features item 10). Window managers like PowerToys
//! FancyZones also move a new window to its last zone, but they size it for a framed window and leave this frameless
//! one a few pixels to the left (user, 2026-11-08): a move in the first seconds after the window shows, made without
//! the mouse, is put back to the saved place.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Manager;

const FILE: &str = "window.json";
/// How long after showing a move that isn't the user's is undone.
/// FancyZones snapped a window shown after an update about 8 s in (user, 11-10).
const SETTLE: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
struct Place { x: i32, y: i32, w: u32, h: u32, #[serde(default)] max: bool }

static SAVED: Mutex<Option<Place>> = Mutex::new(None);
static SHOWN: Mutex<Option<Instant>> = Mutex::new(None);
static GEN: AtomicU64 = AtomicU64::new(0);

fn path() -> std::path::PathBuf { crate::datadir::root().join(FILE) }

/// Before the window first shows: put it where it was, if that is still on a monitor.
pub fn restore(app: &tauri::AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    let Some(p) = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice::<Place>(&b).ok()) else { return };
    let visible = w.available_monitors().unwrap_or_default().iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        let ix = (p.x + p.w as i32).min(mp.x + ms.width as i32) - p.x.max(mp.x);
        let iy = (p.y + p.h as i32).min(mp.y + ms.height as i32) - p.y.max(mp.y);
        ix >= 100 && iy >= 100
    });
    if !visible || p.w < 200 || p.h < 200 { return }
    let _ = w.set_size(tauri::PhysicalSize::new(p.w, p.h));
    let _ = w.set_position(tauri::PhysicalPosition::new(p.x, p.y));
    if p.max { let _ = w.maximize(); }
    *SAVED.lock().expect("winplace") = Some(p);
}

/// The window was just shown: moves in the next few seconds that aren't the user's get undone.
pub fn shown() {
    *SHOWN.lock().expect("winplace") = Some(Instant::now());
}

fn mouse_down() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" { fn GetAsyncKeyState(key: i32) -> i16; }
    unsafe { GetAsyncKeyState(1 /* VK_LBUTTON */) < 0 || GetAsyncKeyState(2 /* VK_RBUTTON */) < 0 }
}

pub fn on_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" || !matches!(event, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) { return }
    if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false) { return }
    let max = window.is_maximized().unwrap_or(false);
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else { return };
    let now = Place { x: pos.x, y: pos.y, w: size.width, h: size.height, max };
    let saved = *SAVED.lock().expect("winplace");
    if max {
        // Keep the place it restores to; just remember it was maximized.
        if let Some(s) = saved { remember(Place { max: true, ..s }) }
        return;
    }
    let settling = SHOWN.lock().expect("winplace").is_some_and(|t| t.elapsed() < SETTLE);
    if let Some(s) = saved.filter(|s| settling && !s.max && *s != now && !mouse_down()) {
        let _ = window.set_size(tauri::PhysicalSize::new(s.w, s.h));
        let _ = window.set_position(tauri::PhysicalPosition::new(s.x, s.y));
        return;
    }
    remember(now);
}

/// Saved half a second after the last move, written whole (temp file, then rename).
fn remember(p: Place) {
    *SAVED.lock().expect("winplace") = Some(p);
    let mine = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        if GEN.load(Ordering::SeqCst) != mine { return }
        let tmp = path().with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&p).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, path());
        }
    });
}
