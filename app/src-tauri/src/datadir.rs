//! Where Obelisk keeps its own data (settings, the sharing key, caches, the window's web data).
//! Default: %LOCALAPPDATA%\dev.obelisk.app. The user can pick another folder in Settings or the installer;
//! the choice lives in `data-location.txt` in the default folder (the only file that stays there):
//! line 1 is the folder, and an optional `move-from=<old folder>` line asks the next start to move the data
//! over before any window opens (nothing is in use then).
//! Profiles' own `.obelisk` folders stay with each profile; `%USERPROFILE%\.obelisk\control.json` stays put
//! because the Minecraft chats look for it there.

use obelisk_core as prism;
use std::path::{Path, PathBuf};

const POINTER: &str = "data-location.txt";

pub fn default_root() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("dev.obelisk.app")
}

fn read_pointer() -> (Option<PathBuf>, Option<PathBuf>) {
    let text = std::fs::read_to_string(default_root().join(POINTER)).unwrap_or_default();
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let dir = lines.next().map(PathBuf::from).filter(|p| p.is_absolute());
    let from = lines.find_map(|l| l.strip_prefix("move-from=")).map(PathBuf::from).filter(|p| p.is_absolute());
    (dir, from)
}

fn write_pointer(dir: &Path, from: Option<&Path>) -> std::io::Result<()> {
    std::fs::create_dir_all(default_root())?;
    let mut text = format!("{}\r\n", dir.display());
    if let Some(f) = from {
        text += &format!("move-from={}\r\n", f.display());
    }
    std::fs::write(default_root().join(POINTER), text)
}

/// A line in data-location.log (next to the pointer), for working out why a move did or didn't happen.
fn log(line: &str) {
    use std::io::Write;
    let p = default_root().join("data-location.log");
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 64_000) {
        let _ = std::fs::remove_file(&p);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{} {line}", prism::timeline::now_ms());
    }
}

/// The folder in use.
pub fn root() -> PathBuf {
    read_pointer().0.unwrap_or_else(default_root)
}

/// At start, before any window: finish a requested move, then point the web view's data there too.
pub fn prepare() {
    let (dir, from) = read_pointer();
    log(&format!("start: dir={dir:?} move-from={from:?} sandboxed={}", super::sandboxed()));
    // Started from inside another app's package, AppData is a redirected copy: never move from there.
    if read_pointer().1.is_some() && !super::sandboxed() && wait_for_others() {
        // A restart can start two copies at once (the second just hands over and quits): one moves, the other waits.
        let lock = default_root().join("moving.lock");
        if std::fs::OpenOptions::new().write(true).create_new(true).open(&lock).is_ok() {
            if let (Some(to), Some(from)) = read_pointer() {
                let moved = from == to || move_contents(&from, &to);
                log(&format!("move {} -> {}: {}", from.display(), to.display(), if moved { "done" } else { "failed, kept the old folder" }));
                // A failed move keeps everything where it was.
                let _ = if moved { write_pointer(&to, None) } else { write_pointer(&from, None) };
            }
            let _ = std::fs::remove_file(&lock);
        } else {
            let t0 = std::time::Instant::now();
            while lock.exists() && t0.elapsed() < std::time::Duration::from_secs(300) {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
    let r = root();
    if r != default_root() {
        // WebView2 keeps its EBWebView folder in here instead of the default.
        unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &r) };
    }
}

/// The Obelisk that asked for the move can take a few seconds to close: wait up to a minute for it.
/// False if one is still running then (leave the move for the next start).
fn wait_for_others() -> bool {
    let t0 = std::time::Instant::now();
    while settled_obelisk_running() {
        if t0.elapsed() > std::time::Duration::from_secs(60) {
            log("another Obelisk kept running; moving on the next start");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    true
}

/// Another Obelisk that has been up a while (not one starting alongside this one).
fn settled_obelisk_running() -> bool {
    let me = std::process::id();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true,
        sysinfo::ProcessRefreshKind::new().with_cmd(sysinfo::UpdateKind::Always));
    // Chats' `obelisk.exe --mcp` helpers run for hours but never touch these files.
    sys.processes().iter().any(|(pid, p)| pid.as_u32() != me && p.name().eq_ignore_ascii_case("obelisk.exe") && p.run_time() > 20
        && !p.cmd().iter().any(|a| a == "--mcp" || a == "--build-id"))
}

/// Copy everything over, then remove the originals. False (and the copy cleaned up) if any file fails.
fn move_contents(from: &Path, to: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(from) else { return true };
    let names: Vec<_> = entries.flatten().map(|e| e.file_name()).filter(|n| n != POINTER && n != "moving.lock" && n != "data-location.log").collect();
    if std::fs::create_dir_all(to).is_err() {
        return false;
    }
    for n in &names {
        if copy_all(&from.join(n), &to.join(n)).is_err() {
            for n in &names {
                let _ = remove(&to.join(n));
            }
            return false;
        }
    }
    for n in &names {
        let _ = remove(&from.join(n));
    }
    true
}

fn copy_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_all(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn remove(p: &Path) -> std::io::Result<()> {
    if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

#[derive(serde::Serialize)]
pub struct Location {
    dir: PathBuf,
    default: PathBuf,
}

#[tauri::command]
pub fn data_dir_get() -> Location {
    Location { dir: root(), default: default_root() }
}

/// Use `dir` from now on: Obelisk closes, moves its data there and starts again.
#[tauri::command]
pub fn data_dir_set(app: tauri::AppHandle, dir: String) -> Result<(), String> {
    let r = request_move(&app, &dir);
    log(&format!("asked to use {dir}: {}", r.as_ref().map(|_| "restarting to move".to_string()).unwrap_or_else(|e| e.clone())));
    r
}

pub fn request_move(app: &tauri::AppHandle, dir: &str) -> Result<(), String> {
    let to = PathBuf::from(dir.trim());
    let from = root();
    if !to.is_absolute() {
        return Err("pick a full folder path".into());
    }
    if to == from {
        return Ok(());
    }
    if to.starts_with(&from) || (from.starts_with(&to) && to != default_root()) {
        return Err("the new folder can't be inside the current one, or the other way round".into());
    }
    let busy = std::fs::read_dir(&to).map(|d| d.flatten().any(|e| e.file_name() != POINTER)).unwrap_or(false);
    if busy {
        return Err(format!("{} isn't empty; pick an empty or new folder", to.display()));
    }
    std::fs::create_dir_all(&to).map_err(|e| format!("can't use {}: {e}", to.display()))?;
    write_pointer(&to, Some(&from)).map_err(|e| e.to_string())?;
    // Start again once this Obelisk has closed (the new one waits for it), so nothing holds the files while they move.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe).args(["--after-pid", &std::process::id().to_string()])
        .spawn().map_err(|e| format!("couldn't restart: {e}"))?;
    super::control::stop();
    app.exit(0);
    Ok(())
}
