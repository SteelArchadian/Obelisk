//! A mod's config files, found and edited from outside the game (like the Configured mod does in game).

use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const ROOTS: [&str; 2] = ["config", "defaultconfigs"];
const EXTS: [&str; 9] = ["toml", "json", "json5", "cfg", "properties", "txt", "yml", "yaml", "snbt"];

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    /// Path relative to the game folder, with forward slashes (e.g. "config/create-client.toml").
    pub path: String,
    pub size: u64,
}

/// Mod ids declared inside a jar (all of them; a jar can hold several mods).
pub fn mod_ids(jar: &Path) -> Vec<String> {
    let Ok(file) = fs::File::open(jar) else { return vec![] };
    ids_in(file, 0)
}

pub(crate) fn ids_in<R: Read + std::io::Seek>(reader: R, depth: u32) -> Vec<String> {
    let Ok(mut zip) = zip::ZipArchive::new(reader) else { return vec![] };
    let mut read = |name: &str| -> Option<String> {
        let mut s = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let mut ids = Vec::new();
    for name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        let Some(doc) = read(name).and_then(|t| t.parse::<toml::Table>().ok()) else { continue };
        for m in doc.get("mods").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(id) = m.get("modId").and_then(|v| v.as_str()) {
                ids.push(id.to_lowercase());
            }
        }
    }
    if let Some(v) = read("fabric.mod.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        if let Some(id) = v["id"].as_str() {
            ids.push(id.to_lowercase());
        }
    }
    // Bundled mods (META-INF/jarjar, META-INF/jars) can own configs too.
    if depth == 0 {
        let nested: Vec<String> = zip.file_names()
            .filter(|n| (n.starts_with("META-INF/jarjar/") || n.starts_with("META-INF/jars/")) && n.ends_with(".jar"))
            .map(str::to_string).collect();
        for n in nested {
            let mut bytes = Vec::new();
            if zip.by_name(&n).ok().and_then(|mut f| f.read_to_end(&mut bytes).ok()).is_some() {
                ids.extend(ids_in(std::io::Cursor::new(bytes), 1));
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

/// Does a config entry (file or folder name directly under config/) belong to one of these mod ids?
pub fn belongs(entry: &str, ids: &[String]) -> bool {
    let e = entry.to_lowercase();
    let stem = e.split('.').next().unwrap_or(&e);
    ids.iter().filter(|id| id.len() >= 3).any(|id| {
        let id_us = id.replace('-', "_");
        let id_dash = id.replace('_', "-");
        [id.as_str(), id_us.as_str(), id_dash.as_str()].iter().any(|i| {
            stem == *i || stem.strip_prefix(i).is_some_and(|rest| rest.starts_with(['-', '_']))
        })
    })
}

/// Config files for the mod in `mods/<file_name>`, newest layout first (config/, then defaultconfigs/).
pub fn find(game: &Path, file_name: &str) -> Result<Vec<ConfigFile>, String> {
    crate::check_name("mods", file_name)?;
    let ids = mod_ids(&game.join("mods").join(file_name));
    let mut out = Vec::new();
    for root in ROOTS {
        let Ok(entries) = fs::read_dir(game.join(root)) else { continue };
        let mut names: Vec<_> = entries.flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let name = e.file_name().to_string_lossy().into_owned();
            if !belongs(&name, &ids) {
                continue;
            }
            collect(game, &e.path(), &mut out, 0);
        }
    }
    // Server configs live inside each world.
    if let Ok(worlds) = fs::read_dir(game.join("saves")) {
        for w in worlds.flatten() {
            let Ok(entries) = fs::read_dir(w.path().join("serverconfig")) else { continue };
            for e in entries.flatten() {
                if belongs(&e.file_name().to_string_lossy(), &ids) {
                    collect(game, &e.path(), &mut out, 0);
                }
            }
        }
    }
    Ok(out)
}

fn collect(game: &Path, path: &Path, out: &mut Vec<ConfigFile>, depth: u32) {
    if path.is_dir() {
        if depth > 3 {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else { return };
        let mut list: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        list.sort();
        for p in list {
            collect(game, &p, out, depth + 1);
        }
    } else if editable(path) {
        let rel = path.strip_prefix(game).unwrap_or(path).to_string_lossy().replace('\\', "/");
        out.push(ConfigFile { path: rel, size: fs::metadata(path).map(|m| m.len()).unwrap_or(0) });
    }
}

fn editable(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTS.contains(&e.to_lowercase().as_str()))
        && fs::metadata(path).map(|m| m.len() < 2 * 1024 * 1024).unwrap_or(false)
}

/// A config path from the UI, checked to stay inside config/ or defaultconfigs/.
fn resolve(game: &Path, rel: &str) -> Result<PathBuf, String> {
    let parts: Vec<&str> = rel.split('/').collect();
    let in_world = parts.len() >= 4 && parts[0] == "saves" && parts[2] == "serverconfig";
    let ok = parts.len() >= 2 && (ROOTS.contains(&parts[0]) || in_world)
        && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.contains(['\\', ':']));
    let path = game.join(rel);
    if !ok || !editable(&path) {
        return Err("not an editable config file".into());
    }
    Ok(path)
}

/// Read a config. The first time Obelisk sees a file it keeps a copy as that file's
/// "default", so "Restore default" can always go back to how it was before any edits.
pub fn read(game: &Path, rel: &str) -> Result<String, String> {
    let path = resolve(game, rel)?;
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let snap = game.join(".obelisk").join("config-defaults").join(rel);
    if !snap.exists() {
        if let Some(dir) = snap.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&snap, &text);
    }
    Ok(text)
}

/// The copy kept the first time this config was opened, if any.
pub fn default_text(game: &Path, rel: &str) -> Result<Option<String>, String> {
    resolve(game, rel)?;
    Ok(fs::read_to_string(game.join(".obelisk").join("config-defaults").join(rel)).ok())
}

/// Put a config back to its kept default (the current version goes to the backups first).
pub fn restore_default(game: &Path, rel: &str) -> Result<String, String> {
    let text = default_text(game, rel)?.ok_or("no saved default for this file")?;
    write(game, rel, &text)?;
    Ok(text)
}

/// Loader and game configs that never belong to a mod jar.
const ALWAYS_OWNED: [&str; 8] = ["neoforge", "forge", "fml", "minecraft", "fabric", "quilt", "jarjar", "obelisk"];

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Entries directly under config/ that no installed mod (enabled or disabled) seems to own.
/// Matching is deliberately loose, so a config is only listed when nothing resembles it.
pub fn orphans(game: &Path) -> Vec<String> {
    let mut owners: Vec<String> = ALWAYS_OWNED.iter().map(|s| s.to_string()).collect();
    if let Ok(mods) = fs::read_dir(game.join("mods")) {
        for m in mods.flatten() {
            let name = m.file_name().to_string_lossy().to_lowercase();
            if !(name.ends_with(".jar") || name.ends_with(".jar.disabled")) {
                continue;
            }
            owners.extend(mod_ids(&m.path()).iter().map(|i| norm(i)));
            // The jar's own name (before the version) as a fallback owner.
            let stem = name.split(['-', '_', '+']).next().unwrap_or("").to_string();
            owners.push(norm(&stem));
        }
    }
    owners.retain(|o| o.len() >= 3);
    owners.sort();
    owners.dedup();
    let Ok(entries) = fs::read_dir(game.join("config")) else { return vec![] };
    let mut out: Vec<String> = entries.flatten().filter_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        let stem = norm(name.split('.').next().unwrap_or(&name));
        if stem.len() < 3 || name.starts_with('.') {
            return None;
        }
        let owned = owners.iter().any(|o| stem.contains(o.as_str()) || o.contains(stem.as_str()));
        (!owned).then_some(name)
    }).collect();
    out.sort_by_key(|n| n.to_lowercase());
    // Second pass: a mod may name its config something unrelated to its id (ETF writes
    // etf_warnings.json). Anything whose name appears inside a mod's code is owned after all.
    let jars: Vec<PathBuf> = fs::read_dir(game.join("mods")).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| { let n = p.to_string_lossy().to_lowercase(); n.ends_with(".jar") || n.ends_with(".jar.disabled") })
        .collect();
    let needles: Vec<(String, Vec<Vec<u8>>)> = out.iter().map(|name| {
        let stem = name.split('.').next().unwrap_or(name).to_string();
        let mut n = vec![name.as_bytes().to_vec()];
        if stem.len() >= 5 && stem != *name {
            n.push(stem.into_bytes());
        }
        (name.clone(), n)
    }).collect();
    let found = std::sync::Mutex::new(std::collections::HashSet::<String>::new());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    std::thread::scope(|sc| {
        for chunk in jars.chunks(jars.len().div_ceil(threads).max(1)) {
            let (needles, found) = (&needles, &found);
            sc.spawn(move || {
                for jar in chunk {
                    scan_jar(jar, needles, found);
                }
            });
        }
    });
    let found = found.into_inner().unwrap_or_default();
    out.retain(|n| !found.contains(n));
    out
}

/// Mark every needle that occurs in the jar's classes or data files.
fn scan_jar(jar: &Path, needles: &[(String, Vec<Vec<u8>>)], found: &std::sync::Mutex<std::collections::HashSet<String>>) {
    let Ok(file) = fs::File::open(jar) else { return };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return };
    let mut buf = Vec::new();
    for i in 0..zip.len() {
        let Ok(mut f) = zip.by_index(i) else { continue };
        let name = f.name().to_lowercase();
        if !(name.ends_with(".class") || name.ends_with(".json") || name.ends_with(".toml")) || f.size() > 8 * 1024 * 1024 {
            continue;
        }
        buf.clear();
        if f.read_to_end(&mut buf).is_err() {
            continue;
        }
        let pending: Vec<&(String, Vec<Vec<u8>>)> = {
            let done = found.lock().unwrap();
            needles.iter().filter(|(n, _)| !done.contains(n)).collect()
        };
        if pending.is_empty() {
            return;
        }
        for (n, pats) in pending {
            if pats.iter().any(|p| contains(&buf, p)) {
                found.lock().unwrap().insert(n.clone());
            }
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.len() >= needle.len() && {
        let first = needle[0];
        hay.iter().enumerate().take(hay.len() - needle.len() + 1)
            .any(|(i, &b)| b == first && &hay[i..i + needle.len()] == needle)
    }
}

/// Remove orphaned entries from config/ (only names `orphans` still reports are touched).
pub fn remove_orphans(game: &Path, names: &[String], remove: impl Fn(&Path) -> Result<(), String>) -> Result<usize, String> {
    let mut n = 0;
    // Only plain entries directly under config/, and never the loader's own files.
    let safe = |x: &String| !x.is_empty() && !x.contains(['/', '\\']) && !x.contains("..") && !x.starts_with('.')
        && !ALWAYS_OWNED.iter().any(|o| norm(x).starts_with(o));
    for name in names.iter().filter(|x| safe(x)) {
        remove(&game.join("config").join(name))?;
        n += 1;
    }
    Ok(n)
}

/// Save a config, keeping the previous version in `.obelisk/config-backups/` (outside config/,
/// so no mod mistakes the backup for a config).
pub fn write(game: &Path, rel: &str, text: &str) -> Result<(), String> {
    let path = resolve(game, rel)?;
    let backup = game.join(".obelisk").join("config-backups").join(rel);
    if let Some(dir) = backup.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::copy(&path, &backup).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("obelisk-tmp");
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orphans_skip_owned_and_loader_configs() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        fs::create_dir_all(g.join("config/jei")).unwrap();
        fs::create_dir_all(g.join("mods")).unwrap();
        for f in ["create-client.toml", "neoforge-client.toml", "fml.toml", "oldmod-common.toml", "jei/jei-client.ini"] {
            fs::write(g.join("config").join(f), "").unwrap();
        }
        fs::write(g.join("mods/create-1.21.1-6.0.jar"), "not a zip").unwrap();
        fs::write(g.join("mods/jei-1.21.1-19.jar.disabled"), "not a zip").unwrap();
        assert_eq!(orphans(g), vec!["oldmod-common.toml".to_string()]);
        let n = remove_orphans(g, &["oldmod-common.toml".into(), "fml.toml".into(), "../x".into()], |p| fs::remove_file(p).map_err(|e| e.to_string())).unwrap();
        assert_eq!(n, 1);
        assert!(g.join("config/fml.toml").exists() && !g.join("config/oldmod-common.toml").exists());
    }

    #[test]
    fn matches_common_config_names() {
        let ids = vec!["create".to_string(), "sophisticated_backpacks".to_string()];
        assert!(belongs("create-client.toml", &ids));
        assert!(belongs("create", &ids));
        assert!(belongs("sophisticated-backpacks-common.toml", &ids));
        assert!(!belongs("createaddition-common.toml", &ids));
        assert!(!belongs("jei", &ids));
        assert!(!belongs("ae2.json", &["ae".to_string()]));
    }

    #[test]
    fn edits_stay_inside_config_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        fs::create_dir_all(g.join("config")).unwrap();
        fs::write(g.join("config/a.toml"), "x = 1\n").unwrap();
        write(g, "config/a.toml", "x = 2\n").unwrap();
        assert_eq!(read(g, "config/a.toml").unwrap(), "x = 2\n");
        assert_eq!(fs::read_to_string(g.join(".obelisk/config-backups/config/a.toml")).unwrap(), "x = 1\n");
        assert!(read(g, "config/../options.txt").is_err());
        assert!(read(g, "mods/a.toml").is_err());
        assert!(read(g, "config/nope.toml").is_err());
        // The first read kept a default; restoring brings it back.
        assert_eq!(default_text(g, "config/a.toml").unwrap().as_deref(), Some("x = 2\n"));
        write(g, "config/a.toml", "x = 3\n").unwrap();
        assert_eq!(restore_default(g, "config/a.toml").unwrap(), "x = 2\n");
    }
}
