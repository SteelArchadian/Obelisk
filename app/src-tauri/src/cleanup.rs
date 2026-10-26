//! What the installer and uninstaller ask of Obelisk itself, so the rules for what is Obelisk's live next to the
//! code that made those files:
//! - `obelisk.exe --reset-data`: a clean install. Obelisk's settings, caches and web data go; modpacks, accounts
//!   and keys stay.
//! - `obelisk.exe --uninstall-cleanup [--keep-data]`: the uninstaller. With `--keep-data` ("Keep my data", the
//!   default) modpacks, instances and settings all stay for a later install; only what points at the removed
//!   program goes (start with Windows, AI assistant connections). Without it everything Obelisk left behind
//!   goes, modpacks to the Recycle Bin, never deleted outright.
//!
//! Obelisk's: its data folder's own files (see OWNED), the pointer folder %LOCALAPPDATA%\dev.obelisk.app,
//! %USERPROFILE%\.obelisk (control API token, test bench), the CurseForge key in Credential Manager, the
//! start-with-Windows entry, and the AI assistant connections it added. Never Obelisk's: Prism Launcher, its
//! accounts and its other files, the Minecraft runtime, anything else the user keeps in a data folder they chose.

use crate::datadir;
use obelisk_core as prism;
use std::path::Path;

/// Files and folders Obelisk itself writes in its data folder.
const OWNED: &[&str] = &["settings.json", "ignored-duplicates.json", "share-key", "hashes", "icons", "EBWebView",
    "data-location.log", "moving.lock", "whats-new.json"];
/// Of those, what a clean install keeps: the sharing identity (friends know this PC by it).
const KEPT_ON_RESET: &[&str] = &["share-key"];

fn remove(p: &Path) {
    let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
}

/// Clean install: fresh settings and caches.
pub fn reset_data() {
    let root = datadir::root();
    for name in OWNED.iter().filter(|n| !KEPT_ON_RESET.contains(n)) {
        remove(&root.join(name));
    }
}

/// Uninstall. Keeping the data leaves modpacks and settings untouched; otherwise everything Obelisk left behind
/// goes, modpacks to the Recycle Bin.
pub fn uninstall(keep_data: bool) -> Result<(), String> {
    if let Ok(run) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", winreg::enums::KEY_SET_VALUE) {
        let _ = run.delete_value("Obelisk");
    }
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from);
    if let Some(home) = &home {
        forget_assistants(home);
    }
    if keep_data {
        return Ok(());
    }
    let instances = prism::instances_dir(&prism::data_dir());
    if instances.is_dir() {
        trash::delete(&instances).map_err(|e| format!("couldn't move the modpacks to the Recycle Bin: {e}"))?;
    }
    // Item by item, never the whole folder: modpacks or the user's own files may live in it ("Keep them with
    // Obelisk's data" puts Instances there). A folder goes only once it's empty.
    for dir in [datadir::root(), datadir::default_root()] {
        for name in OWNED.iter().chain(&["data-location.txt"]) {
            remove(&dir.join(name));
        }
        let _ = std::fs::remove_dir(&dir);
    }
    if let Some(home) = &home {
        remove(&home.join(".obelisk"));
    }
    if let Ok(e) = keyring::Entry::new("Obelisk", "curseforge-api-key") {
        let _ = e.delete_credential();
    }
    Ok(())
}

/// Take out only the "obelisk" MCP server entries Settings → Plugins → Connect added; the rest is the user's.
fn forget_assistants(home: &Path) {
    let claude = home.join(".claude.json");
    if let Some(mut cfg) = std::fs::read(&claude).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
        if cfg.pointer_mut("/mcpServers").and_then(|s| s.as_object_mut()).and_then(|s| s.remove("obelisk")).is_some() {
            if let Ok(text) = serde_json::to_vec_pretty(&cfg) {
                let _ = std::fs::write(&claude, text);
            }
        }
    }
    let codex = home.join(".codex").join("config.toml");
    if let Ok(text) = std::fs::read_to_string(&codex) {
        let mut out = Vec::new();
        let mut skipping = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                skipping = t == "[mcp_servers.obelisk]" || t.starts_with("[mcp_servers.obelisk.");
            }
            if !skipping {
                out.push(line);
            }
        }
        if out.len() != text.lines().count() {
            let _ = std::fs::write(&codex, out.join("\n") + "\n");
        }
    }
}
