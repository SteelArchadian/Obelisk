//! Per-PC settings for a profile (pc-overrides.json in Obelisk's data folder), applied before every launch: a weaker
//! PC keeps its own lighter settings even when the pack and its options came from another PC.
//!
//! ```json
//! { "profiles": { "*": { ... }, "<profile or test copy name>": {
//!     "options": { "enableVsync": false, "renderClouds": "\"fast\"" },
//!     "files": [ { "file": "config/sodium-options.json", "key": "performance.chunk_builder_threads", "value": 2 },
//!                { "file": "config/superluminal-client.toml", "key": "rayTracing.enabled", "value": false } ] } } }
//! ```
//! `options` go into options.txt as written (a text value is put in as is, so options.txt's own quotes are part of it).
//! `files` set one key in a .toml (dotted section.key; it must already be there) or a .json file (dotted path).
//! Each file is copied to `<name>.obelisk-pc-backup` the first time an override changes it.

use serde_json::Value;
use std::fs;
use std::path::Path;

pub const FILE: &str = "pc-overrides.json";

/// The overrides that apply to `name` ("*" first, then the profile's own, which wins).
pub fn for_profile(all: &Value, name: &str) -> (Vec<(String, String)>, Vec<(String, String, Value)>) {
    let mut options: Vec<(String, String)> = Vec::new();
    let mut files: Vec<(String, String, Value)> = Vec::new();
    for key in ["*", name] {
        let Some(p) = all.pointer("/profiles").and_then(|p| p.get(key)) else { continue };
        for (k, v) in p["options"].as_object().into_iter().flatten() {
            let v = match v { Value::String(s) => s.clone(), other => other.to_string() };
            options.retain(|(o, _)| o != k);
            options.push((k.clone(), v));
        }
        for f in p["files"].as_array().into_iter().flatten() {
            let (Some(file), Some(k)) = (f["file"].as_str(), f["key"].as_str()) else { continue };
            files.retain(|(a, b, _)| !(a == file && b == k));
            files.push((file.to_string(), k.to_string(), f["value"].clone()));
        }
    }
    (options, files)
}

/// Check an overrides document before it's saved: what's wrong with it, if anything.
pub fn validate(all: &Value) -> Result<(), String> {
    let profiles = all.get("profiles").and_then(Value::as_object).ok_or("needs a \"profiles\" object")?;
    for (name, p) in profiles {
        for (k, v) in p.get("options").and_then(Value::as_object).into_iter().flatten() {
            if k.is_empty() || k.contains([':', '\n', '\r']) { return Err(format!("{name}: {k:?} isn't an options.txt key")) }
            if matches!(v, Value::Array(_) | Value::Object(_) | Value::Null) || v.as_str().is_some_and(|s| s.contains(['\n', '\r'])) {
                return Err(format!("{name}: option {k}: give a text, number or true/false"));
            }
        }
        for f in p.get("files").and_then(Value::as_array).into_iter().flatten() {
            let file = f["file"].as_str().ok_or(format!("{name}: a files entry needs \"file\""))?;
            f["key"].as_str().filter(|k| !k.is_empty()).ok_or(format!("{name}: {file} needs \"key\""))?;
            inside(file).ok_or(format!("{name}: {file} must be a path inside the profile"))?;
            let lower = file.to_ascii_lowercase();
            if !lower.ends_with(".toml") && !lower.ends_with(".json") { return Err(format!("{name}: {file}: only .toml and .json files")) }
        }
    }
    Ok(())
}

fn inside(file: &str) -> Option<&Path> {
    let p = Path::new(file);
    p.components().all(|c| matches!(c, std::path::Component::Normal(_))).then_some(p)
}

/// Keep the file as it was before the first override touched it.
fn backup_once(path: &Path) {
    let mut bak = path.as_os_str().to_owned();
    bak.push(".obelisk-pc-backup");
    let bak = std::path::PathBuf::from(bak);
    if path.is_file() && !bak.exists() {
        let _ = fs::copy(path, bak);
    }
}

/// Apply them to the game folder. Returns what couldn't be applied (the launch goes ahead either way).
pub fn apply(game: &Path, options: &[(String, String)], files: &[(String, String, Value)]) -> Vec<String> {
    let mut problems = Vec::new();
    if !options.is_empty() {
        let path = game.join("options.txt");
        let text = fs::read_to_string(&path).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        for (k, v) in options {
            let want = format!("{k}:{v}");
            match lines.iter().position(|l| l.split(':').next() == Some(k)) {
                Some(i) => lines[i] = want,
                None => lines.push(want),
            }
        }
        let out = lines.join("\n") + "\n";
        if out != text {
            backup_once(&path);
            if let Err(e) = fs::write(&path, out) { problems.push(format!("options.txt: {e}")) }
        }
    }
    for (file, key, value) in files {
        let Some(rel) = inside(file) else { problems.push(format!("{file}: not inside the profile")); continue };
        let path = game.join(rel);
        let Ok(text) = fs::read_to_string(&path) else { problems.push(format!("{file}: not there yet")); continue };
        let new = if file.to_ascii_lowercase().ends_with(".toml") {
            let v = match value { Value::String(s) => toml::Value::String(s.clone()).to_string(), other => other.to_string() };
            crate::tomlset::set_key(&text, key, &v)
        } else {
            set_json(&text, key, value)
        };
        match new {
            Ok(new) if new != text => {
                backup_once(&path);
                if let Err(e) = fs::write(&path, new) { problems.push(format!("{file}: {e}")) }
            }
            Ok(_) => {}
            Err(e) => problems.push(format!("{file}: {e}")),
        }
    }
    problems
}

/// Set a dotted key in JSON text (it must already be there, like TOML keys). Written back pretty-printed.
fn set_json(text: &str, key: &str, value: &Value) -> Result<String, String> {
    let mut doc: Value = serde_json::from_str(text).map_err(|e| format!("isn't valid JSON ({e})"))?;
    let mut at = &mut doc;
    for part in key.split('.') {
        at = at.get_mut(part).ok_or(format!("{key} isn't in it"))?;
    }
    if at == value {
        return Ok(text.to_string());
    }
    *at = value.clone();
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overrides_apply_once_and_keep_a_backup() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        fs::create_dir_all(g.join("config")).unwrap();
        fs::write(g.join("options.txt"), "renderDistance:6\nenableVsync:true\n").unwrap();
        fs::write(g.join("config/sodium-options.json"), "{\n  \"performance\": {\n    \"chunk_builder_threads\": 0\n  }\n}").unwrap();
        fs::write(g.join("config/superluminal-client.toml"), "[rayTracing]\n\t#on or off\n\tenabled = true\n").unwrap();
        let all = json!({ "profiles": {
            "*": { "options": { "renderDistance": 12 } },
            "td": { "options": { "enableVsync": false, "renderClouds": "\"fast\"", "renderDistance": 6 },
                    "files": [ { "file": "config/sodium-options.json", "key": "performance.chunk_builder_threads", "value": 2 },
                               { "file": "config/superluminal-client.toml", "key": "rayTracing.enabled", "value": false } ] } } });
        validate(&all).unwrap();
        let (o, f) = for_profile(&all, "td");
        assert!(apply(g, &o, &f).is_empty());
        let opts = fs::read_to_string(g.join("options.txt")).unwrap();
        assert_eq!(opts, "renderDistance:6\nenableVsync:false\nrenderClouds:\"fast\"\n", "the profile's own value wins over *");
        assert!(fs::read_to_string(g.join("config/sodium-options.json")).unwrap().contains("\"chunk_builder_threads\": 2"));
        assert_eq!(fs::read_to_string(g.join("config/superluminal-client.toml")).unwrap(), "[rayTracing]\n\t#on or off\n\tenabled = false\n");
        assert_eq!(fs::read_to_string(g.join("options.txt.obelisk-pc-backup")).unwrap(), "renderDistance:6\nenableVsync:true\n");
        // Again: nothing changes, and the backup still holds the original.
        assert!(apply(g, &o, &f).is_empty());
        assert_eq!(fs::read_to_string(g.join("options.txt.obelisk-pc-backup")).unwrap(), "renderDistance:6\nenableVsync:true\n");
        assert!(validate(&json!({ "profiles": { "x": { "files": [ { "file": "../a.toml", "key": "a" } ] } } })).is_err());
    }
}
