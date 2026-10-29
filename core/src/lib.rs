//! Read-only view of Prism Launcher's data folder: instances, their settings and content.
//! Prism stays the engine (accounts, Java, launching); Obelisk is the face.

pub mod configs;
pub mod crash;
pub mod curseforge;
pub mod dupes;
pub mod guard;
pub mod held;
pub mod instdir;
pub mod packcode;
pub mod lightning;
pub mod migrate;
pub mod modrinth;
pub mod insight;
pub mod sides;
pub mod window;
pub mod history;
pub mod java;
pub mod timeline;
pub mod tomlset;
pub mod packtrash;
pub mod share;
pub mod share_net;
pub mod trash;
pub mod updates;
pub mod work;
pub mod testworld;
pub mod crosssite;

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Prism's data folder. `OBELISK_PRISM_DIR` overrides it (used by tests).
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OBELISK_PRISM_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("PrismLauncher")
}

/// Prism's executable in its default per-user install location.
pub fn launcher_exe() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
        .join("Programs")
        .join("PrismLauncher")
        .join("prismlauncher.exe")
}

/// Parse Prism's INI-style config files (`key=value`, `[General]` sections ignored). Prism writes them with Qt,
/// which escapes a backslash as `\\`; that is undone here.
pub fn read_cfg(path: &Path) -> HashMap<String, String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter(|l| !l.starts_with('[') && !l.starts_with(';'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').replace("\\\\", "\\")))
        .collect()
}

/// A value as Prism (Qt) reads it back. Qt treats `\` in these files as an escape, so `D:\Modding` would come back
/// as `D:odding`: paths get forward slashes (as Prism writes them itself), any other backslash is doubled.
fn cfg_value(v: &str) -> String {
    let b = v.as_bytes();
    let path = (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || v.starts_with("\\\\");
    if path { v.replace('\\', "/") } else { v.replace('\\', "\\\\") }
}

/// The profile id in a running game's "-Djava.library.path=<instances folder>/<id>/natives" argument (Prism sets
/// it). The instances folder can be anywhere, in any case.
pub fn natives_instance(arg: &str) -> Option<String> {
    let path = arg.strip_prefix("-Djava.library.path=")?.replace('\\', "/");
    let dir = path.trim_end_matches('/').strip_suffix("/natives")?;
    dir.rsplit('/').next().filter(|id| !id.is_empty()).map(String::from)
}

/// Repair path settings an older Obelisk wrote with bare backslashes (which Prism misreads, so it found no
/// packs or no Java). Returns whether anything changed.
pub fn heal_cfg(path: &Path) -> Result<bool, String> {
    let Ok(text) = fs::read_to_string(path) else { return Ok(false) };
    let mut fixes = vec![];
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if matches!(k, "InstanceDir" | "JavaPath" | "IconsDir" | "CentralModsDir" | "JavaDir" | "SkinsDir")
            && v.contains('\\') && !v.contains("\\\\") && !v.starts_with('"') {
            fixes.push((k, Some(v.to_string())));
        }
    }
    if fixes.is_empty() { return Ok(false) }
    update_cfg(path, &fixes)?;
    Ok(true)
}

pub fn instances_dir(data: &Path) -> PathBuf {
    let dir = read_cfg(&data.join("prismlauncher.cfg"))
        .get("InstanceDir")
        .cloned()
        .unwrap_or_else(|| "instances".into());
    let dir = PathBuf::from(dir);
    if dir.is_absolute() { dir } else { data.join(dir) }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub icon_key: String,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: String,
    pub seconds_played: u64,
    pub last_launch_ms: u64,
}

const LOADERS: [(&str, &str); 5] = [
    ("net.neoforged", "NeoForge"),
    ("net.minecraftforge", "Forge"),
    ("net.fabricmc.fabric-loader", "Fabric"),
    ("org.quiltmc.quilt-loader", "Quilt"),
    ("com.mumfrey.liteloader", "LiteLoader"),
];

pub fn list_instances(data: &Path) -> Vec<Instance> {
    let Ok(entries) = fs::read_dir(instances_dir(data)) else { return vec![] };
    let mut out: Vec<Instance> = entries
        .flatten()
        .filter(|e| e.path().join("instance.cfg").is_file())
        .map(|e| read_instance(&e.path()))
        .collect();
    out.sort_by(|a, b| b.last_launch_ms.cmp(&a.last_launch_ms).then(a.name.cmp(&b.name)));
    out
}

fn read_instance(dir: &Path) -> Instance {
    let cfg = read_cfg(&dir.join("instance.cfg"));
    let id = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut inst = Instance {
        name: cfg.get("name").cloned().unwrap_or_else(|| id.clone()),
        icon_key: cfg.get("iconKey").cloned().unwrap_or_default(),
        seconds_played: cfg.get("totalTimePlayed").and_then(|v| v.parse().ok()).unwrap_or(0),
        last_launch_ms: cfg.get("lastLaunchTime").and_then(|v| v.parse().ok()).unwrap_or(0),
        id,
        minecraft: String::new(),
        loader: "Vanilla".into(),
        loader_version: String::new(),
    };
    let pack: serde_json::Value = fs::read_to_string(dir.join("mmc-pack.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for c in pack["components"].as_array().into_iter().flatten() {
        let uid = c["uid"].as_str().unwrap_or_default();
        let version = c["version"].as_str().unwrap_or_default().to_string();
        if uid == "net.minecraft" {
            inst.minecraft = version;
        } else if let Some((_, label)) = LOADERS.iter().find(|(u, _)| *u == uid) {
            inst.loader = label.to_string();
            inst.loader_version = version;
        }
    }
    inst
}

/// The `.minecraft` folder of an instance (Prism uses `minecraft` or `.minecraft`).
pub fn game_dir(data: &Path, instance_id: &str) -> Option<PathBuf> {
    if instance_id.contains(['/', '\\']) || instance_id.contains("..") {
        return None;
    }
    let root = instances_dir(data).join(instance_id);
    ["minecraft", ".minecraft"].iter().map(|d| root.join(d)).find(|p| p.is_dir())
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContentItem {
    pub file_name: String,
    pub name: String,
    pub version: String,
    pub authors: String,
    pub enabled: bool,
    /// "curseforge", "modrinth" or "local"
    pub source: String,
    pub project_id: String,
    /// When the file landed in this profile (file creation time).
    pub installed_ms: u64,
    /// When the file was last changed.
    pub modified_ms: u64,
}

pub const KINDS: [&str; 3] = ["mods", "resourcepacks", "shaderpacks"];

/// Every file in an instance content folder, enriched from Prism's `.index` metadata
/// and (for mods) the jar's own manifest.
pub fn list_content(game: &Path, kind: &str) -> Vec<ContentItem> {
    let folder = game.join(kind);
    let index = read_index(&folder.join(".index"));
    let Ok(entries) = fs::read_dir(&folder) else { return vec![] };
    let mut items: Vec<ContentItem> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let file_name = e.file_name().to_string_lossy().into_owned();
            let enabled = !file_name.ends_with(".disabled");
            let base = file_name.trim_end_matches(".disabled").to_string();
            if kind == "shaderpacks" && base.ends_with(".txt") {
                return None; // Iris per-pack settings, not content
            }
            let meta = index.get(&base);
            let jar = if base.ends_with(".jar") { cached_jar_meta(&e.path()) } else { JarMeta::default() };
            let fm = e.metadata().ok();
            let ms = |t: Option<std::time::SystemTime>| t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
            let modified_ms = ms(fm.as_ref().and_then(|m| m.modified().ok()));
            let installed_ms = ms(fm.as_ref().and_then(|m| m.created().ok())).max(1).min(modified_ms.max(1));
            Some(ContentItem {
                installed_ms,
                modified_ms,
                name: meta.map(|m| m.name.clone()).filter(|n| !n.is_empty())
                    .or(jar.name).unwrap_or_else(|| base.clone()),
                version: jar.version.unwrap_or_default(),
                authors: jar.authors.unwrap_or_default(),
                source: meta.map(|m| m.source.clone()).unwrap_or_else(|| "local".into()),
                project_id: meta.map(|m| m.project_id.clone()).unwrap_or_default(),
                enabled,
                file_name,
            })
        })
        .collect();
    items.sort_by_key(|i| i.name.to_lowercase());
    items
}

struct IndexEntry {
    name: String,
    source: String,
    project_id: String,
}

/// Prism keeps packwiz-style `<slug>.pw.toml` files per tracked mod.
fn read_index(dir: &Path) -> HashMap<String, IndexEntry> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir(dir) else { return map };
    for e in entries.flatten() {
        let Ok(text) = fs::read_to_string(e.path()) else { continue };
        let Ok(doc) = text.parse::<toml::Table>() else { continue };
        let Some(file) = doc.get("filename").and_then(|v| v.as_str()) else { continue };
        let update = doc.get("update").and_then(|v| v.as_table());
        let (source, project_id) = match update {
            Some(u) if u.contains_key("curseforge") => (
                "curseforge",
                u["curseforge"].get("project-id").map(|v| v.to_string()).unwrap_or_default(),
            ),
            Some(u) if u.contains_key("modrinth") => (
                "modrinth",
                u["modrinth"].get("mod-id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            ),
            _ => ("local", String::new()),
        };
        map.insert(file.to_string(), IndexEntry {
            name: doc.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            source: source.into(),
            project_id,
        });
    }
    map
}

#[derive(Default, Clone)]
struct JarMeta {
    name: Option<String>,
    version: Option<String>,
    authors: Option<String>,
}

/// read_jar_meta, remembered per file (size + modified time) so reopening a profile doesn't reopen every jar.
fn cached_jar_meta(path: &Path) -> JarMeta {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (u64, std::time::SystemTime, JarMeta)>>> = OnceLock::new();
    let Ok(m) = fs::metadata(path) else { return JarMeta::default() };
    let stamp = (m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH));
    let cache = CACHE.get_or_init(Default::default);
    if let Some((len, t, meta)) = cache.lock().unwrap().get(path) {
        if (*len, *t) == stamp { return meta.clone(); }
    }
    let meta = read_jar_meta(path);
    cache.lock().unwrap().insert(path.to_path_buf(), (stamp.0, stamp.1, meta.clone()));
    meta
}

/// Display name, version and authors from a NeoForge/Forge mods.toml or a fabric.mod.json.
fn read_jar_meta(path: &Path) -> JarMeta {
    let Ok(file) = fs::File::open(path) else { return JarMeta::default() };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return JarMeta::default() };
    let mut read = |name: &str| -> Option<String> {
        let mut s = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    for toml_name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(text) = read(toml_name) {
            let Ok(doc) = text.parse::<toml::Table>() else { continue };
            let Some(m) = doc.get("mods").and_then(|v| v.as_array()).and_then(|a| a.first()) else { continue };
            let s = |k: &str| m.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let version = s("version").filter(|v| !v.contains("${"));
            let authors = s("authors").or_else(|| doc.get("authors").and_then(|v| v.as_str()).map(str::to_string));
            return JarMeta { name: s("displayName"), version, authors };
        }
    }
    if let Some(text) = read("fabric.mod.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let authors = v["authors"].as_array().map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().or_else(|| x["name"].as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            return JarMeta {
                name: v["name"].as_str().map(str::to_string),
                version: v["version"].as_str().map(str::to_string),
                authors,
            };
        }
    }
    JarMeta::default()
}

pub(crate) fn check_name(kind: &str, file_name: &str) -> Result<(), String> {
    if !(KINDS.contains(&kind) || kind == "datapacks") || file_name.is_empty() || file_name.contains(['/', '\\']) || file_name.contains("..") {
        return Err("invalid content path".into());
    }
    Ok(())
}

/// Icon/logo bytes inside a mod jar or pack zip: mods.toml `logoFile`, fabric `icon`, or `pack.png`.
fn read_icon(path: &Path) -> Option<Vec<u8>> {
    if path.is_dir() {
        return fs::read(path.join("pack.png")).ok();
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
    let mut text = |name: &str| -> Option<String> {
        let mut s = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let mut candidates = Vec::new();
    for toml_name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        let Some(doc) = text(toml_name).and_then(|t| t.parse::<toml::Table>().ok()) else { continue };
        let first = doc.get("mods").and_then(|v| v.as_array()).and_then(|a| a.first());
        for logo in [first.and_then(|m| m.get("logoFile")), doc.get("logoFile")].into_iter().flatten() {
            if let Some(s) = logo.as_str() {
                candidates.push(s.trim_start_matches('/').to_string());
            }
        }
    }
    if let Some(v) = text("fabric.mod.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        match &v["icon"] {
            serde_json::Value::String(s) => candidates.push(s.clone()),
            serde_json::Value::Object(sizes) => {
                // pick the largest size
                if let Some((_, s)) = sizes.iter().max_by_key(|(k, _)| k.parse::<u32>().unwrap_or(0)) {
                    candidates.extend(s.as_str().map(str::to_string));
                }
            }
            _ => {}
        }
    }
    candidates.push("pack.png".into());
    for name in candidates {
        let Ok(mut entry) = zip.by_name(&name) else { continue };
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_ok() && !bytes.is_empty() {
            return Some(bytes);
        }
    }
    None
}

/// Extract each content file's icon into `cache` (once per file version) and map file name -> icon path.
pub fn content_icons(game: &Path, kind: &str, cache: &Path) -> HashMap<String, PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut out = HashMap::new();
    let Ok(entries) = fs::read_dir(game.join(kind)) else { return out };
    let _ = fs::create_dir_all(cache);
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.ends_with(".txt") {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (name.trim_end_matches(".disabled"), meta.len(), meta.modified().ok()).hash(&mut h);
        let key = format!("{:016x}", h.finish());
        let icon = cache.join(format!("{key}.png"));
        let none = cache.join(format!("{key}.none"));
        if icon.is_file() {
            out.insert(name, icon);
        } else if !none.exists() {
            match read_icon(&path) {
                Some(bytes) if fs::write(&icon, &bytes).is_ok() => { out.insert(name, icon); }
                _ => { let _ = fs::write(&none, b""); }
            }
        }
    }
    out
}

/// Before a launch: the pack starts with Iris shaders off (a shader pack can drop a weak PC to a few frames a second;
/// they turn back on in-game with K or Video Settings). Only `enableShaders` changes; the chosen shader pack and every
/// other line stay. A missing file is left missing (no shaders chosen). Returns whether anything changed.
pub fn shaders_off(game: &Path) -> Result<bool, String> {
    let path = game.join("config").join("iris.properties");
    let Ok(text) = fs::read_to_string(&path) else { return Ok(false) };
    let mut changed = false;
    let lines: Vec<String> = text.lines().map(|l| {
        let key = l.split(['=', ':']).next().unwrap_or("").trim();
        if key == "enableShaders" && l.split_once(['=', ':']).is_some_and(|(_, v)| v.trim() != "false") {
            changed = true;
            "enableShaders=false".to_string()
        } else { l.to_string() }
    }).collect();
    if !changed { return Ok(false) }
    let mut out = lines.join(if text.contains("\r\n") { "\r\n" } else { "\n" });
    if text.ends_with('\n') { out.push_str(if text.contains("\r\n") { "\r\n" } else { "\n" }) }
    // Written beside it and swapped in, so a crash mid-write can't leave Iris's settings half-written.
    let tmp = path.with_extension("properties.obelisk-tmp");
    fs::write(&tmp, out).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Set (`Some`) or remove (`None`) keys in a Prism INI file, keeping every other line as is.
pub fn update_cfg(path: &Path, updates: &[(&str, Option<String>)]) -> Result<(), String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if !lines.iter().any(|l| l.trim() == "[General]") {
        lines.insert(0, "[General]".into());
    }
    for (key, value) in updates {
        let pos = lines.iter().position(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == *key));
        match (pos, value) {
            (Some(i), Some(v)) => lines[i] = format!("{key}={}", cfg_value(v)),
            (Some(i), None) => { lines.remove(i); }
            (None, Some(v)) => lines.push(format!("{key}={}", cfg_value(v))),
            (None, None) => {}
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(path, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

fn instance_root(data: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        return Err("invalid instance id".into());
    }
    let root = instances_dir(data).join(id);
    if root.join("instance.cfg").is_file() { Ok(root) } else { Err(format!("instance {id} not found")) }
}

/// Per-profile settings Obelisk edits (memory, Java args, window).
#[derive(Serialize, serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileOptions {
    pub name: String,
    pub override_memory: bool,
    pub max_mem_mb: u32,
    pub min_mem_mb: u32,
    pub override_java_args: bool,
    pub jvm_args: String,
    pub override_window: bool,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    /// Run this profile with a chosen Java (javaw path) instead of Prism's pick.
    pub override_java_location: bool,
    pub java_path: String,
}

const fn bool_str(b: bool) -> &'static str { if b { "true" } else { "false" } }

pub fn profile_options(data: &Path, id: &str) -> Result<ProfileOptions, String> {
    let cfg = read_cfg(&instance_root(data, id)?.join("instance.cfg"));
    let num = |k: &str, d: u32| cfg.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let flag = |k: &str| cfg.get(k).is_some_and(|v| v == "true");
    Ok(ProfileOptions {
        name: cfg.get("name").cloned().unwrap_or_else(|| id.to_string()),
        override_memory: flag("OverrideMemory"),
        max_mem_mb: num("MaxMemAlloc", 4096),
        min_mem_mb: num("MinMemAlloc", 512),
        override_java_args: flag("OverrideJavaArgs"),
        jvm_args: cfg.get("JvmArgs").cloned().unwrap_or_default(),
        override_window: flag("OverrideWindow"),
        width: num("MinecraftWinWidth", 854),
        height: num("MinecraftWinHeight", 480),
        maximized: flag("LaunchMaximized"),
        override_java_location: flag("OverrideJavaLocation"),
        java_path: cfg.get("JavaPath").cloned().unwrap_or_default(),
    })
}

pub fn set_profile_options(data: &Path, id: &str, o: &ProfileOptions) -> Result<(), String> {
    let path = instance_root(data, id)?.join("instance.cfg");
    let name = o.name.trim();
    update_cfg(&path, &[
        ("name", Some(if name.is_empty() { id.to_string() } else { name.to_string() })),
        ("OverrideMemory", Some(bool_str(o.override_memory).into())),
        ("MaxMemAlloc", Some(o.max_mem_mb.max(512).to_string())),
        ("MinMemAlloc", Some(o.min_mem_mb.clamp(256, o.max_mem_mb.max(512)).to_string())),
        ("OverrideJavaArgs", Some(bool_str(o.override_java_args).into())),
        ("JvmArgs", Some(o.jvm_args.replace(['\r', '\n'], " ").trim().to_string())),
        ("OverrideWindow", Some(bool_str(o.override_window).into())),
        ("MinecraftWinWidth", Some(o.width.max(320).to_string())),
        ("MinecraftWinHeight", Some(o.height.max(240).to_string())),
        ("LaunchMaximized", Some(bool_str(o.maximized).into())),
        ("OverrideJavaLocation", Some(bool_str(o.override_java_location && !o.java_path.trim().is_empty()).into())),
        ("JavaPath", Some(o.java_path.trim().to_string())),
    ])
}

/// Copy a whole instance under a new name. Returns the new id.
pub fn clone_instance(data: &Path, id: &str, new_name: &str) -> Result<String, String> {
    let src = instance_root(data, id)?;
    let instances = instances_dir(data);
    let base: String = new_name.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let base = if base.trim().is_empty() { format!("{id} copy") } else { base.trim().to_string() };
    let new_id = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} ({i})") })
        .find(|n| !instances.join(n).exists()).unwrap();
    let dst = instances.join(&new_id);
    let total = count_files(&src);
    copy_dir(&src, &dst, &mut 0, total)?;
    update_cfg(&dst.join("instance.cfg"), &[
        ("name", Some(new_name.trim().to_string())),
        ("ObeliskSource", None),
        ("totalTimePlayed", Some("0".into())),
        ("lastLaunchTime", Some("0".into())),
    ])?;
    Ok(new_id)
}

/// A clone starts with no backup history of its own, and isn't collaborative (a shared pack's identity and key
/// belong to the original: two packs with one identity answer friends with each other's key).
fn clone_skips(dir: &Path, name: &std::ffi::OsStr) -> bool {
    (name == "history" || name == "share") && dir.file_name().is_some_and(|n| n == ".obelisk")
}

fn count_files(dir: &Path) -> usize {
    fs::read_dir(dir).into_iter().flatten().flatten()
        .filter(|e| !clone_skips(dir, &e.file_name()))
        .map(|e| if e.path().is_dir() { count_files(&e.path()) } else { 1 })
        .sum()
}

fn copy_dir(src: &Path, dst: &Path, done: &mut usize, total: usize) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        if clone_skips(src, &e.file_name()) {
            continue;
        }
        let to = dst.join(e.file_name());
        if e.path().is_dir() { copy_dir(&e.path(), &to, done, total)?; } else {
            fs::copy(e.path(), &to).map_err(|e| e.to_string())?;
            *done += 1;
            work::progress("", *done, total);
        }
    }
    Ok(())
}

/// Move an instance into the modpack trash (see `packtrash`); it can be restored for a while.
pub fn delete_instance(data: &Path, id: &str) -> Result<packtrash::TrashedPack, String> {
    instance_root(data, id)?;
    let name = list_instances(data).into_iter().find(|i| i.id == id).map(|i| i.name).unwrap_or_else(|| id.to_string());
    packtrash::put(&instances_dir(data), id, &name)
}

/// Prism's global defaults Obelisk exposes in Settings.
#[derive(Serialize, serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrismDefaults {
    pub max_mem_mb: u32,
    pub min_mem_mb: u32,
    pub jvm_args: String,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

pub fn prism_defaults(data: &Path) -> PrismDefaults {
    let cfg = read_cfg(&data.join("prismlauncher.cfg"));
    let num = |k: &str, d: u32| cfg.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    PrismDefaults {
        max_mem_mb: num("MaxMemAlloc", 4096),
        min_mem_mb: num("MinMemAlloc", 512),
        jvm_args: cfg.get("JvmArgs").cloned().unwrap_or_default(),
        width: num("MinecraftWinWidth", 854),
        height: num("MinecraftWinHeight", 480),
        maximized: cfg.get("LaunchMaximized").is_some_and(|v| v == "true"),
    }
}

pub fn set_prism_defaults(data: &Path, d: &PrismDefaults) -> Result<(), String> {
    update_cfg(&data.join("prismlauncher.cfg"), &[
        ("MaxMemAlloc", Some(d.max_mem_mb.max(512).to_string())),
        ("MinMemAlloc", Some(d.min_mem_mb.clamp(256, d.max_mem_mb.max(512)).to_string())),
        ("JvmArgs", Some(d.jvm_args.replace(['\r', '\n'], " ").trim().to_string())),
        ("MinecraftWinWidth", Some(d.width.max(320).to_string())),
        ("MinecraftWinHeight", Some(d.height.max(240).to_string())),
        ("LaunchMaximized", Some(bool_str(d.maximized).into())),
    ])
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    format!("{:x}", Sha1::digest(bytes))
}

/// SHA-1 of every content file in a folder, cached in `cache/hashes.json` by name, size and mtime.
pub fn file_sha1s(game: &Path, kind: &str, cache: &Path) -> HashMap<String, String> {
    use sha1::{Digest, Sha1};
    let cache_file = cache.join("hashes.json");
    let mut known: HashMap<String, String> = fs::read_to_string(&cache_file).ok()
        .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut out = HashMap::new();
    let mut dirty = false;
    let Ok(entries) = fs::read_dir(game.join(kind)) else { return out };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() || name.starts_with('.') {
            continue;
        }
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        let key = format!("{}|{}|{}|{mtime}", game.join(kind).display(), name, meta.len());
        let sha = match known.get(&key) {
            Some(s) => s.clone(),
            None => {
                let Ok(bytes) = fs::read(e.path()) else { continue };
                let s = format!("{:x}", Sha1::digest(&bytes));
                known.insert(key, s.clone());
                dirty = true;
                s
            }
        };
        out.insert(name, sha);
    }
    if dirty {
        let _ = fs::create_dir_all(cache);
        let _ = fs::write(&cache_file, serde_json::to_string(&known).unwrap_or_default());
    }
    out
}

/// Delete a content file plus its Prism `.index` entry. `remove` does the deleting
/// (the app sends files to the recycle bin; tests delete outright).
pub fn delete_content(game: &Path, kind: &str, file_name: &str, remove: impl Fn(&Path) -> Result<(), String>) -> Result<(), String> {
    check_name(kind, file_name)?;
    let folder = game.join(kind);
    let path = folder.join(file_name);
    if !path.exists() {
        return Err(format!("{file_name} not found"));
    }
    remove(&path)?;
    for p in index_files_for(&folder, file_name) {
        remove(&p)?;
    }
    Ok(())
}

/// Prism `.index` entries that track this file (enabled or disabled).
pub fn index_files_for(folder: &Path, file_name: &str) -> Vec<PathBuf> {
    let base = file_name.trim_end_matches(".disabled");
    let Ok(entries) = fs::read_dir(folder.join(".index")) else { return vec![] };
    entries.flatten().map(|e| e.path()).filter(|p| {
        fs::read_to_string(p).ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
            .and_then(|d| d.get("filename").and_then(|v| v.as_str()).map(|f| f == base))
            .unwrap_or(false)
    }).collect()
}

fn modified_ms(path: &Path) -> u64 {
    fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct World {
    pub folder: String,
    pub last_played_ms: u64,
    pub icon: Option<PathBuf>,
    /// Data packs in the world's `datapacks/` folder.
    pub datapacks: Vec<String>,
}

/// A world folder inside a profile, checked so a name can't point anywhere else.
pub fn world_dir(game: &Path, world: &str) -> Result<PathBuf, String> {
    if world.is_empty() || world.contains(['/', '\\']) || world.contains("..") {
        return Err("invalid world".into());
    }
    let dir = game.join("saves").join(world);
    if !dir.join("level.dat").is_file() {
        return Err(format!("world {world} not found"));
    }
    Ok(dir)
}

fn list_datapacks(world: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(world.join("datapacks")) else { return vec![] };
    let mut packs: Vec<String> = entries.flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    packs.sort_by_key(|n| n.to_lowercase());
    packs
}

/// Worlds in `saves/`, most recently played first.
pub fn list_worlds(game: &Path) -> Vec<World> {
    let Ok(entries) = fs::read_dir(game.join("saves")) else { return vec![] };
    let mut worlds: Vec<World> = entries.flatten()
        .map(|e| e.path())
        .filter(|p| p.join("level.dat").is_file())
        .map(|p| World {
            folder: p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            last_played_ms: modified_ms(&p.join("level.dat")),
            icon: Some(p.join("icon.png")).filter(|i| i.is_file()),
            datapacks: list_datapacks(&p),
        })
        .collect();
    worlds.sort_by(|a, b| b.last_played_ms.cmp(&a.last_played_ms));
    worlds
}

/// Screenshot paths, newest first.
pub fn list_screenshots(game: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(game.join("screenshots")) else { return vec![] };
    let mut shots: Vec<(u64, PathBuf)> = entries.flatten()
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("png" | "jpg" | "jpeg")))
        .map(|p| (modified_ms(&p), p))
        .collect();
    shots.sort_by(|a, b| b.0.cmp(&a.0));
    shots.into_iter().map(|(_, p)| p).collect()
}

/// Plain-text logs in `logs/`, newest first.
pub fn list_logs(game: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(game.join("logs")) else { return vec![] };
    let mut logs: Vec<(u64, String)> = entries.flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".log"))
        .map(|e| (modified_ms(&e.path()), e.file_name().to_string_lossy().into_owned()))
        .collect();
    logs.sort_by(|a, b| b.0.cmp(&a.0));
    logs.into_iter().map(|(_, n)| n).collect()
}

/// The last `max_bytes` of a log, starting at a line boundary.
pub fn read_log_tail(game: &Path, name: &str, max_bytes: u64) -> Result<String, String> {
    use std::io::{Seek, SeekFrom};
    if !name.ends_with(".log") || name.contains(['/', '\\']) || name.contains("..") {
        return Err("invalid log name".into());
    }
    let mut f = fs::File::open(game.join("logs").join(name)).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(if start > 0 { text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or(text) } else { text })
}

/// Enable or disable a content file by adding or removing Prism's `.disabled` suffix.
pub fn set_enabled(game: &Path, kind: &str, file_name: &str, enabled: bool) -> Result<String, String> {
    check_name(kind, file_name)?;
    let folder = game.join(kind);
    let base = file_name.trim_end_matches(".disabled");
    let (from, to) = if enabled {
        (format!("{base}.disabled"), base.to_string())
    } else {
        (base.to_string(), format!("{base}.disabled"))
    };
    if !folder.join(&from).is_file() {
        return if folder.join(&to).is_file() { Ok(to) } else { Err(format!("{file_name} not found")) };
    }
    fs::rename(folder.join(&from), folder.join(&to)).map_err(|e| e.to_string())?;
    Ok(to)
}

/// Give a profile its own picture (a real PNG, JPEG, GIF or WebP up to 1 MB, so it can travel with Share).
/// It goes into Prism's icons folder under a new key; the old picture file is left alone (other profiles may use it).
pub fn set_icon_base64(data: &Path, id: &str, b64: &str) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()).map_err(|_| "the picture didn't come through")?;
    set_icon(data, id, &bytes)
}

pub fn set_icon(data: &Path, id: &str, bytes: &[u8]) -> Result<String, String> {
    let ext = packcode::image_ext(bytes).ok_or("That isn't a picture Obelisk can use (PNG, JPEG, GIF or WebP).")?;
    if bytes.len() > packcode::ICON_MAX {
        return Err("That picture is over 1 MB.".into());
    }
    let root = instance_root(data, id)?;
    let slug: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let key = format!("obelisk-{slug}-{}", &sha1_hex(bytes)[..8]);
    let icons = icons_dir(data);
    fs::create_dir_all(&icons).map_err(|e| e.to_string())?;
    fs::write(icons.join(format!("{key}.{ext}")), bytes).map_err(|e| e.to_string())?;
    update_cfg(&root.join("instance.cfg"), &[("iconKey", Some(key.clone()))])?;
    Ok(key)
}

/// Prism's icons folder: its IconsDir setting (relative to its data folder unless absolute), else `icons`.
pub fn icons_dir(data: &Path) -> PathBuf {
    match read_cfg(&data.join("prismlauncher.cfg")).get("IconsDir").map(|d| d.trim().trim_matches('"')).filter(|d| !d.is_empty()) {
        Some(d) => data.join(d),
        None => data.join("icons"),
    }
}

/// Instance icon as a data URL, from Prism's icons folder (custom keys only).
pub fn icon_data_url(data: &Path, icon_key: &str) -> Option<String> {
    use base64::Engine;
    if icon_key.is_empty() || icon_key.contains(['/', '\\', '.']) {
        return None;
    }
    // Icons Obelisk wrote before it followed IconsDir are in `icons`.
    for dir in [icons_dir(data), data.join("icons")] {
        for ext in ["png", "jpg", "jpeg", "gif", "webp"] {
            if let Ok(bytes) = fs::read(dir.join(format!("{icon_key}.{ext}"))) {
                let mime = if ext == "jpg" { "jpeg" } else { ext };
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                return Some(format!("data:image/{mime};base64,{b64}"));
            }
        }
    }
    None
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub name: String,
    pub uuid: String,
}

/// The active Minecraft account in Prism (name and UUID only; tokens are never read out).
pub fn active_account(data: &Path) -> Option<Account> {
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(data.join("accounts.json")).ok()?).ok()?;
    let accounts = v["accounts"].as_array()?;
    let a = accounts.iter().find(|a| a["active"].as_bool() == Some(true)).or(accounts.first())?;
    Some(Account {
        name: a["profile"]["name"].as_str()?.to_string(),
        uuid: a["profile"]["id"].as_str().unwrap_or_default().to_string(),
    })
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountEntry {
    pub name: String,
    pub uuid: String,
    pub active: bool,
    /// "msa" or "offline"
    pub kind: String,
}

fn read_accounts(data: &Path) -> Result<serde_json::Value, String> {
    let text = fs::read_to_string(data.join("accounts.json")).map_err(|e| format!("accounts.json: {e}"))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Write accounts.json atomically, keeping one backup of the previous file.
fn write_accounts(data: &Path, v: &serde_json::Value) -> Result<(), String> {
    let path = data.join("accounts.json");
    let tmp = data.join("accounts.json.obelisk-tmp");
    let _ = fs::copy(&path, data.join("accounts.json.obelisk-bak"));
    fs::write(&tmp, serde_json::to_string_pretty(v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Accounts Prism knows about (names and ids only; tokens are left untouched).
pub fn list_accounts(data: &Path) -> Vec<AccountEntry> {
    let Ok(v) = read_accounts(data) else { return vec![] };
    v["accounts"].as_array().into_iter().flatten().filter_map(|a| Some(AccountEntry {
        name: a["profile"]["name"].as_str()?.to_string(),
        uuid: a["profile"]["id"].as_str().unwrap_or_default().to_string(),
        active: a["active"].as_bool() == Some(true),
        kind: a["type"].as_str().unwrap_or("msa").to_lowercase(),
    })).collect()
}

pub fn set_active_account(data: &Path, uuid: &str) -> Result<(), String> {
    let mut v = read_accounts(data)?;
    let list = v["accounts"].as_array_mut().ok_or("no accounts")?;
    if !list.iter().any(|a| a["profile"]["id"] == uuid) {
        return Err("account not found".into());
    }
    for a in list.iter_mut() {
        if let Some(obj) = a.as_object_mut() {
            if obj.get("profile").and_then(|p| p.get("id")).and_then(|i| i.as_str()) == Some(uuid) {
                obj.insert("active".into(), serde_json::Value::Bool(true));
            } else {
                obj.remove("active");
            }
        }
    }
    write_accounts(data, &v)
}

/// Log out: drop the account (and its saved sign-in) from Prism.
pub fn remove_account(data: &Path, uuid: &str) -> Result<(), String> {
    let mut v = read_accounts(data)?;
    let list = v["accounts"].as_array_mut().ok_or("no accounts")?;
    let before = list.len();
    let was_active = list.iter().any(|a| a["profile"]["id"] == uuid && a["active"].as_bool() == Some(true));
    list.retain(|a| a["profile"]["id"] != uuid);
    if list.len() == before {
        return Err("account not found".into());
    }
    if was_active {
        if let Some(obj) = list.first_mut().and_then(|a| a.as_object_mut()) {
            obj.insert("active".into(), serde_json::Value::Bool(true));
        }
    }
    write_accounts(data, &v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_profile_in_natives_path() {
        use super::natives_instance as n;
        assert_eq!(n("-Djava.library.path=D:/Modding/Minecraft/Obelisk/Instances/Foundation (EXPERIMENTAL)/natives").as_deref(), Some("Foundation (EXPERIMENTAL)"));
        assert_eq!(n(r"-Djava.library.path=C:\Users\a\AppData\Roaming\PrismLauncher\instances\Odyssey\natives").as_deref(), Some("Odyssey"));
        assert_eq!(n("-Xmx4G"), None);
    }

    #[test]
    fn cfg_paths_survive_qt() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("prismlauncher.cfg");
        std::fs::write(&p, "[General]\nInstanceDir=D:\\Modding\\Instances\nJvmArgs=-Xmx4G\n").unwrap();
        assert!(super::heal_cfg(&p).unwrap());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("InstanceDir=D:/Modding/Instances"), "{text}");
        assert!(!super::heal_cfg(&p).unwrap());
        super::update_cfg(&p, &[("JavaPath", Some(r"C:\Java\bin\javaw.exe".into())), ("Note", Some(r"a\b".into()))]).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("JavaPath=C:/Java/bin/javaw.exe") && text.contains(r"Note=a\\b"), "{text}");
        assert_eq!(super::read_cfg(&p)["Note"], r"a\b");
        assert_eq!(super::instances_dir(dir.path()), std::path::PathBuf::from("D:/Modding/Instances"));
    }

    use super::*;

    #[test]
    fn shaders_start_off_keeping_the_rest() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path().join("config");
        fs::create_dir_all(&cfg).unwrap();
        assert!(!shaders_off(t.path()).unwrap());
        fs::write(cfg.join("iris.properties"), "#Iris\r\nshaderPack=Photon.zip\r\nenableShaders=true\r\n").unwrap();
        assert!(shaders_off(t.path()).unwrap());
        assert_eq!(fs::read_to_string(cfg.join("iris.properties")).unwrap(), "#Iris\r\nshaderPack=Photon.zip\r\nenableShaders=false\r\n");
        assert!(!shaders_off(t.path()).unwrap());
    }
    use std::io::Write;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn jar(path: &Path, entries: &[(&str, &str)]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, body) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().to_path_buf();
        let inst = data.join("instances").join("Foundation-prism");
        write(&inst.join("instance.cfg"),
              "[General]\nname=Foundation\niconKey=foundation\ntotalTimePlayed=7200\nlastLaunchTime=1700000000000\n");
        write(&inst.join("mmc-pack.json"), r#"{"components":[
            {"uid":"net.minecraft","version":"1.21.1"},
            {"uid":"net.neoforged","version":"21.1.252"}]}"#);
        write(&data.join("instances").join("Old").join("instance.cfg"), "name=Old Pack\n");
        (tmp, data)
    }

    #[test]
    fn lists_instances_newest_first_with_versions() {
        let (_t, data) = fixture();
        let list = list_instances(&data);
        assert_eq!(list.len(), 2);
        let f = &list[0];
        assert_eq!((f.id.as_str(), f.name.as_str()), ("Foundation-prism", "Foundation"));
        assert_eq!((f.minecraft.as_str(), f.loader.as_str(), f.loader_version.as_str()),
                   ("1.21.1", "NeoForge", "21.1.252"));
        assert_eq!(f.seconds_played, 7200);
        assert_eq!(list[1].loader, "Vanilla");
    }

    #[test]
    fn honours_custom_instance_dir() {
        let (_t, data) = fixture();
        let elsewhere = data.join("elsewhere");
        fs::rename(data.join("instances"), &elsewhere).unwrap();
        write(&data.join("prismlauncher.cfg"), &format!("InstanceDir={}\n", elsewhere.display()));
        assert_eq!(list_instances(&data).len(), 2);
    }

    #[test]
    fn content_merges_index_and_jar_metadata() {
        let (_t, data) = fixture();
        let game = data.join("instances/Foundation-prism/minecraft");
        jar(&game.join("mods/tracked.jar"), &[("META-INF/neoforge.mods.toml",
            "[[mods]]\nmodId='t'\ndisplayName='Jar Name'\nversion='1.2.3'\nauthors='Serilum'\n")]);
        write(&game.join("mods/.index/tracked.pw.toml"),
              "name = 'Tracked Mod'\nfilename = 'tracked.jar'\n[update.curseforge]\nfile-id = 5\nproject-id = 42\n");
        jar(&game.join("mods/fab.jar.disabled"), &[("fabric.mod.json",
            r#"{"name":"Fab","version":"0.9","authors":["a",{"name":"b"}]}"#)]);
        jar(&game.join("mods/mydev.jar"), &[("META-INF/neoforge.mods.toml",
            "[[mods]]\nmodId='d'\nversion='${file.jarVersion}'\n")]);
        let items = list_content(&game, "mods");
        let by = |f: &str| items.iter().find(|i| i.file_name == f).unwrap().clone();

        let t = by("tracked.jar");
        assert_eq!((t.name.as_str(), t.version.as_str(), t.authors.as_str()), ("Tracked Mod", "1.2.3", "Serilum"));
        assert_eq!((t.source.as_str(), t.project_id.as_str(), t.enabled), ("curseforge", "42", true));

        let f = by("fab.jar.disabled");
        assert_eq!((f.name.as_str(), f.authors.as_str(), f.enabled), ("Fab", "a, b", false));

        let d = by("mydev.jar");
        assert_eq!((d.name.as_str(), d.version.as_str(), d.source.as_str()), ("mydev.jar", "", "local"));
    }

    #[test]
    fn shader_settings_files_are_hidden() {
        let (_t, data) = fixture();
        let game = data.join("instances/Foundation-prism/minecraft");
        write(&game.join("shaderpacks/bliss.zip"), "x");
        write(&game.join("shaderpacks/bliss.zip.txt"), "x");
        let names: Vec<_> = list_content(&game, "shaderpacks").into_iter().map(|i| i.file_name).collect();
        assert_eq!(names, ["bliss.zip"]);
    }

    const PNG: &str = "\u{89}PNG fake";

    #[test]
    fn icons_come_from_logo_file_fabric_icon_or_pack_png_and_are_cached() {
        let (_t, data) = fixture();
        let game = data.join("instances/Foundation-prism/minecraft");
        let cache = data.join("cache");
        jar(&game.join("mods/neo.jar"), &[
            ("META-INF/neoforge.mods.toml", "[[mods]]\nmodId='n'\nlogoFile='logo.png'\n"),
            ("logo.png", PNG)]);
        jar(&game.join("mods/fab.jar.disabled"), &[
            ("fabric.mod.json", r#"{"icon":{"16":"small.png","128":"assets/big.png"}}"#),
            ("small.png", "small"), ("assets/big.png", "big")]);
        jar(&game.join("mods/bare.jar"), &[("META-INF/neoforge.mods.toml", "[[mods]]\nmodId='b'\n")]);
        jar(&game.join("resourcepacks/pack.zip"), &[("pack.png", "rp")]);

        let icons = content_icons(&game, "mods", &cache);
        assert_eq!(fs::read_to_string(&icons["neo.jar"]).unwrap(), PNG);
        assert_eq!(fs::read_to_string(&icons["fab.jar.disabled"]).unwrap(), "big");
        assert!(!icons.contains_key("bare.jar"));
        let rp = content_icons(&game, "resourcepacks", &cache);
        assert_eq!(fs::read_to_string(&rp["pack.zip"]).unwrap(), "rp");

        // Second call is served from the cache, including the "no icon" result.
        let files_before = fs::read_dir(&cache).unwrap().count();
        assert_eq!(content_icons(&game, "mods", &cache).len(), 2);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), files_before);
    }

    #[test]
    fn delete_removes_file_and_its_index_entry_only() {
        let (_t, data) = fixture();
        let game = data.join("instances/Foundation-prism/minecraft");
        write(&game.join("mods/a.jar.disabled"), "x");
        write(&game.join("mods/b.jar"), "x");
        write(&game.join("mods/.index/a.pw.toml"), "filename = 'a.jar'\n");
        write(&game.join("mods/.index/b.pw.toml"), "filename = 'b.jar'\n");
        let rm = |p: &Path| fs::remove_file(p).map_err(|e| e.to_string());
        delete_content(&game, "mods", "a.jar.disabled", rm).unwrap();
        assert!(!game.join("mods/a.jar.disabled").exists());
        assert!(!game.join("mods/.index/a.pw.toml").exists());
        assert!(game.join("mods/b.jar").exists() && game.join("mods/.index/b.pw.toml").exists());
        assert!(delete_content(&game, "mods", "a.jar", rm).is_err());
        assert!(delete_content(&game, "mods", "..\\b.jar", rm).is_err());
    }

    #[test]
    fn worlds_screenshots_and_logs() {
        let (_t, data) = fixture();
        let game = data.join("instances/Foundation-prism/minecraft");
        write(&game.join("saves/Alpha/level.dat"), "x");
        write(&game.join("saves/Alpha/icon.png"), "x");
        write(&game.join("saves/not-a-world/readme.txt"), "x");
        write(&game.join("screenshots/a.png"), "x");
        write(&game.join("screenshots/notes.txt"), "x");
        write(&game.join("logs/latest.log"), "line1\nline2\nline3\n");
        write(&game.join("logs/old.log.gz"), "x");

        let worlds = list_worlds(&game);
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].folder, "Alpha");
        assert!(worlds[0].icon.is_some());
        assert_eq!(list_screenshots(&game).len(), 1);
        assert_eq!(list_logs(&game), ["latest.log"]);
        assert_eq!(read_log_tail(&game, "latest.log", 1000).unwrap(), "line1\nline2\nline3\n");
        assert_eq!(read_log_tail(&game, "latest.log", 9).unwrap(), "line3\n"); // cut to a line start
        assert!(read_log_tail(&game, "../x.log", 10).is_err());
    }

    #[test]
    fn cfg_update_keeps_other_lines_and_options_round_trip() {
        let (_t, data) = fixture();
        let cfg = data.join("instances/Foundation-prism/instance.cfg");
        update_cfg(&cfg, &[("JvmArgs", Some("-Xss2m".into())), ("iconKey", None)]).unwrap();
        let read = read_cfg(&cfg);
        assert_eq!(read["JvmArgs"], "-Xss2m");
        assert_eq!(read["totalTimePlayed"], "7200");
        assert!(!read.contains_key("iconKey"));

        let mut o = profile_options(&data, "Foundation-prism").unwrap();
        assert_eq!(o.name, "Foundation");
        o.override_memory = true;
        o.max_mem_mb = 6000;
        o.jvm_args = "-XX:+UseZGC\n-XX:+ZGenerational".into();
        set_profile_options(&data, "Foundation-prism", &o).unwrap();
        let back = profile_options(&data, "Foundation-prism").unwrap();
        assert!(back.override_memory);
        assert_eq!(back.max_mem_mb, 6000);
        assert_eq!(back.jvm_args, "-XX:+UseZGC -XX:+ZGenerational");
        assert!(profile_options(&data, "..\\x").is_err());

        let mut d = prism_defaults(&data);
        d.max_mem_mb = 8192;
        set_prism_defaults(&data, &d).unwrap();
        assert_eq!(prism_defaults(&data).max_mem_mb, 8192);
    }

    #[test]
    fn clone_and_delete() {
        let (_t, data) = fixture();
        write(&data.join("instances/Foundation-prism/minecraft/mods/a.jar"), "x");
        update_cfg(&data.join("instances/Foundation-prism/instance.cfg"), &[("ObeliskSource", Some("D:/cf".into()))]).unwrap();
        let id = clone_instance(&data, "Foundation-prism", "Foundation Test").unwrap();
        assert_eq!(id, "Foundation Test");
        assert!(data.join("instances/Foundation Test/minecraft/mods/a.jar").is_file());
        let cfg = read_cfg(&data.join("instances/Foundation Test/instance.cfg"));
        assert_eq!(cfg["name"], "Foundation Test");
        assert!(!cfg.contains_key("ObeliskSource"));
        assert_eq!(clone_instance(&data, "Foundation-prism", "Foundation Test").unwrap(), "Foundation Test (2)");
        let t = delete_instance(&data, "Foundation Test").unwrap();
        assert!(!data.join("instances/Foundation Test").exists());
        assert_eq!(packtrash::list(&instances_dir(&data))[0].id, t.id);
        assert!(delete_instance(&data, "nope").is_err());
    }

    #[test]
    fn switch_and_log_out_accounts_keep_tokens_and_back_up() {
        let (_t, data) = fixture();
        write(&data.join("accounts.json"), r#"{"formatVersion":3,"accounts":[
            {"type":"MSA","profile":{"id":"u1","name":"Alt"},"msa":{"refresh_token":"t1"}},
            {"type":"MSA","active":true,"profile":{"id":"u2","name":"Main"},"msa":{"refresh_token":"t2"}}]}"#);
        set_active_account(&data, "u1").unwrap();
        let list = list_accounts(&data);
        assert!(list[0].active && !list[1].active);
        assert!(data.join("accounts.json.obelisk-bak").is_file());
        remove_account(&data, "u1").unwrap(); // removing the active one promotes the next
        let list = list_accounts(&data);
        assert_eq!(list.len(), 1);
        assert!(list[0].active);
        let raw = fs::read_to_string(data.join("accounts.json")).unwrap();
        assert!(raw.contains("\"t2\"") && !raw.contains("\"t1\""));
        assert!(remove_account(&data, "nope").is_err());
    }

    #[test]
    fn active_account_prefers_active_flag() {
        let (_t, data) = fixture();
        write(&data.join("accounts.json"), r#"{"formatVersion":3,"accounts":[
            {"profile":{"id":"u1","name":"Alt"}},
            {"active":true,"profile":{"id":"u2","name":"Player"},"msa":{"refresh_token":"secret"}}]}"#);
        assert_eq!(active_account(&data), Some(Account { name: "Player".into(), uuid: "u2".into() }));
        fs::remove_file(data.join("accounts.json")).unwrap();
        assert_eq!(active_account(&data), None);
    }

    #[test]
    fn toggling_renames_and_rejects_bad_paths() {
        let (_t, data) = fixture();
        let game = data.join("instances/Foundation-prism/minecraft");
        write(&game.join("mods/a.jar"), "x");
        assert_eq!(set_enabled(&game, "mods", "a.jar", false).unwrap(), "a.jar.disabled");
        assert!(game.join("mods/a.jar.disabled").is_file());
        assert_eq!(set_enabled(&game, "mods", "a.jar.disabled", true).unwrap(), "a.jar");
        assert_eq!(set_enabled(&game, "mods", "a.jar", true).unwrap(), "a.jar"); // already enabled
        assert!(set_enabled(&game, "mods", "../a.jar", true).is_err());
        assert!(set_enabled(&game, "saves", "a.jar", true).is_err());
        assert!(game_dir(&data, "..\\x").is_none());
    }
}

/// Pinned content in a profile (keys like "modrinth:AANobbMI", "curseforge:238222" or a file name):
/// Update all leaves these alone.
pub fn pins(game: &Path) -> Vec<String> {
    fs::read(game.join(".obelisk").join("pins.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn set_pins(game: &Path, pins: &[String]) -> Result<(), String> {
    let dir = game.join(".obelisk");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("pins.json"), serde_json::to_vec_pretty(pins).unwrap()).map_err(|e| e.to_string())
}
