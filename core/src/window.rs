//! A profile's game window: windowed, borderless or fullscreen. Borderless goes through whichever
//! mod the pack has for it (Cubes Without Borders, Borderless Mining), so F11 and the in-game option
//! keep working; without one, Obelisk makes the window borderless itself once the game opens.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Windowed,
    Borderless,
    Fullscreen,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WindowMode {
    pub mode: Mode,
    /// The mod that handles borderless here; None means Obelisk does it.
    pub via: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Support {
    Cwb,
    BorderlessMining,
}

fn support(game: &Path) -> Option<Support> {
    let mods = fs::read_dir(game.join("mods")).ok()?;
    for e in mods.flatten() {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if !n.ends_with(".jar") {
            continue;
        }
        if n.starts_with("cwb-") || n.starts_with("cubes-without-borders") || n.starts_with("cubeswithoutborders") {
            return Some(Support::Cwb);
        }
        if n.starts_with("borderlessmining") || n.starts_with("borderless-mining") {
            return Some(Support::BorderlessMining);
        }
    }
    None
}

const CWB: &str = "config/cubes_without_borders.json";
const BM: &str = "config/borderlessmining.json";
/// Obelisk's own note, for packs without a borderless mod.
const OURS: &str = ".obelisk/window.json";

fn read_json(p: &Path) -> Value {
    fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| Value::Object(Default::default()))
}

fn write_json(p: &Path, v: &Value) -> Result<(), String> {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::write(p, serde_json::to_string_pretty(v).unwrap()).map_err(|e| e.to_string())
}

fn vanilla_fullscreen(game: &Path) -> bool {
    fs::read_to_string(game.join("options.txt")).ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("fullscreen:").map(|v| v.trim() == "true")))
        .unwrap_or(false)
}

/// Sets `fullscreen:` in options.txt. A missing options.txt is left alone: the game (or a
/// default-options mod) writes the first one.
fn set_vanilla_fullscreen(game: &Path, on: bool) -> Result<(), String> {
    let p = game.join("options.txt");
    let Ok(s) = fs::read_to_string(&p) else { return Ok(()) };
    let mut found = false;
    let mut out: Vec<String> = s.lines().map(|l| {
        if l.starts_with("fullscreen:") {
            found = true;
            format!("fullscreen:{on}")
        } else {
            l.to_string()
        }
    }).collect();
    if !found {
        out.push(format!("fullscreen:{on}"));
    }
    fs::write(&p, out.join("\n") + "\n").map_err(|e| e.to_string())
}

pub fn get(game: &Path) -> WindowMode {
    let sup = support(game);
    let mode = match sup {
        Some(Support::Cwb) => {
            let v = read_json(&game.join(CWB));
            match v["fullscreenMode"].as_str() {
                Some("BORDERLESS") => Mode::Borderless,
                Some("ON") => Mode::Fullscreen,
                _ if vanilla_fullscreen(game) && v["preferredFullscreenMode"] == "BORDERLESS" => Mode::Borderless,
                _ if vanilla_fullscreen(game) => Mode::Fullscreen,
                _ => Mode::Windowed,
            }
        }
        Some(Support::BorderlessMining) => match (vanilla_fullscreen(game), read_json(&game.join(BM))["enabled"].as_bool().unwrap_or(true)) {
            (true, true) => Mode::Borderless,
            (true, false) => Mode::Fullscreen,
            _ => Mode::Windowed,
        },
        None => match read_json(&game.join(OURS))["mode"].as_str() {
            Some("borderless") => Mode::Borderless,
            _ if vanilla_fullscreen(game) => Mode::Fullscreen,
            _ => Mode::Windowed,
        },
    };
    let via = sup.map(|s| match s {
        Support::Cwb => "Cubes Without Borders",
        Support::BorderlessMining => "Borderless Mining",
    }.to_string());
    WindowMode { mode, via }
}

pub fn set(game: &Path, mode: Mode) -> Result<(), String> {
    let ours = game.join(OURS);
    match support(game) {
        Some(Support::Cwb) => {
            let p = game.join(CWB);
            let mut v = read_json(&p);
            v["fullscreenMode"] = match mode { Mode::Windowed => "OFF", Mode::Borderless => "BORDERLESS", Mode::Fullscreen => "ON" }.into();
            if mode != Mode::Windowed {
                // F11 toggles into the same kind of fullscreen.
                v["preferredFullscreenMode"] = match mode { Mode::Borderless => "BORDERLESS", _ => "ON" }.into();
            }
            write_json(&p, &v)?;
            // CWB rebuilds its mode from options.txt at startup: fullscreen:false resets it to OFF.
            set_vanilla_fullscreen(game, mode != Mode::Windowed)?;
        }
        Some(Support::BorderlessMining) => {
            let p = game.join(BM);
            let mut v = read_json(&p);
            if mode != Mode::Windowed {
                v["enabled"] = (mode == Mode::Borderless).into();
                write_json(&p, &v)?;
            }
            set_vanilla_fullscreen(game, mode != Mode::Windowed)?;
        }
        None => {
            set_vanilla_fullscreen(game, mode == Mode::Fullscreen)?;
        }
    }
    // Only meaningful without a mod, but keep it in step either way.
    if mode == Mode::Borderless {
        write_json(&ours, &serde_json::json!({ "mode": "borderless" }))
    } else {
        let _ = fs::remove_file(&ours);
        Ok(())
    }
}

/// Whether Obelisk should make the window borderless itself after the game opens.
pub fn obelisk_borderless(game: &Path) -> bool {
    support(game).is_none() && get(game).mode == Mode::Borderless
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwb_round_trip() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        fs::create_dir_all(g.join("mods")).unwrap();
        fs::write(g.join("mods/cwb-neoforge-3.0.0+mc1.21.jar"), "").unwrap();
        fs::write(g.join("options.txt"), "version:3955\nfullscreen:false\n").unwrap();
        assert_eq!(get(g).mode, Mode::Windowed);
        set(g, Mode::Borderless).unwrap();
        assert_eq!(get(g).mode, Mode::Borderless);
        assert_eq!(get(g).via.as_deref(), Some("Cubes Without Borders"));
        assert!(!obelisk_borderless(g));
        assert!(fs::read_to_string(g.join("options.txt")).unwrap().contains("fullscreen:true"), "or CWB resets to OFF");
        set(g, Mode::Fullscreen).unwrap();
        assert_eq!(get(g).mode, Mode::Fullscreen);
        assert!(fs::read_to_string(g.join("options.txt")).unwrap().contains("fullscreen:true"));
    }

    #[test]
    fn no_mod_uses_obelisk() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        set(g, Mode::Borderless).unwrap();
        assert!(obelisk_borderless(g));
        set(g, Mode::Windowed).unwrap();
        assert_eq!(get(g).mode, Mode::Windowed);
    }
}
