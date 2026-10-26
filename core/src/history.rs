//! Backups that store changes, not copies: every backup is a list of the pack's files with their
//! content hash, and each file's content is kept once in a shared store (`.obelisk/history/objects`).
//! Jars and zips are hard-linked into the store when possible, so they cost no space until the pack
//! drops or replaces them. Worlds, logs and screenshots are not part of backups.
//!
//! Each backup also records what changed since the one before it, which is the changelog the UI shows.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Folders (under the game folder) that make up "the pack".
pub(crate) const TRACKED: [&str; 9] = ["mods", "config", "defaultconfigs", "kubejs", "scripts", "resourcepacks", "shaderpacks", "global_packs", "datapacks"];
/// Single files under the game folder.
const TRACKED_FILES: [&str; 1] = ["options.txt"];
/// Prism's loader/version file, one level up from the game folder.
pub(crate) const PACK_FILE: &str = "@instance/mmc-pack.json";
/// Skip anything this big (it's not pack content).
const MAX_FILE: u64 = 512 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub mtime_ms: u64,
    pub hash: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// "added", "removed" or "changed"
    pub kind: String,
    /// "mods", "configs", "resourcepacks", "shaderpacks", "scripts", "loader" or "other"
    pub area: String,
    pub path: String,
    /// Friendly text, e.g. "Create 6.0.1 → 6.0.2".
    pub label: String,
    /// For a changed config file, the settings that changed ("render.distance: 8 → 12").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub at_ms: u64,
    /// "auto", "manual" or "safety" (taken by Obelisk right before something risky)
    pub kind: String,
    pub label: String,
    pub files: usize,
    pub bytes: u64,
    /// What changed since the previous backup.
    pub changes: Vec<Change>,
}

pub(crate) fn root(game: &Path) -> PathBuf {
    game.join(".obelisk").join("history")
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn mtime_ms(m: &fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Turn a stored path back into a real one.
pub(crate) fn real(game: &Path, rel: &str) -> Option<PathBuf> {
    if rel.contains("..") || rel.starts_with('/') || rel.contains(':') && !rel.starts_with("@instance/") {
        return None;
    }
    Some(match rel.strip_prefix("@instance/") {
        Some(r) => game.parent()?.join(r),
        None => game.join(rel),
    })
}

fn area_of(path: &str) -> &'static str {
    match path.split('/').next().unwrap_or("") {
        "mods" => "mods",
        "config" | "defaultconfigs" => "configs",
        "resourcepacks" => "resourcepacks",
        "shaderpacks" => "shaderpacks",
        "kubejs" | "scripts" => "scripts",
        "@instance" => "loader",
        _ => "other",
    }
}

/// Every pack file right now (no hashing yet): path -> (size, mtime).
fn scan(game: &Path) -> BTreeMap<String, (u64, u64)> {
    let mut out = BTreeMap::new();
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, (u64, u64)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            // Caches and tool folders inside mods/ (".connector", ".fabric"...), but keep Prism's ".index".
            if name.starts_with('.') && name != ".index" {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let r = format!("{rel}/{name}");
            if meta.is_dir() {
                walk(&e.path(), &r, out);
            } else if meta.len() <= MAX_FILE {
                out.insert(r, (meta.len(), mtime_ms(&meta)));
            }
        }
    }
    for d in TRACKED {
        walk(&game.join(d), d, &mut out);
    }
    for f in TRACKED_FILES {
        if let Ok(m) = fs::metadata(game.join(f)) {
            out.insert(f.to_string(), (m.len(), mtime_ms(&m)));
        }
    }
    if let Some(m) = game.parent().and_then(|p| fs::metadata(p.join("mmc-pack.json")).ok()) {
        out.insert(PACK_FILE.to_string(), (m.len(), mtime_ms(&m)));
    }
    out
}

pub(crate) fn hash_file(p: &Path) -> Result<String, String> {
    use sha1::{Digest, Sha1};
    let mut f = fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

pub(crate) fn object(game: &Path, hash: &str) -> PathBuf {
    root(game).join("objects").join(&hash[..2]).join(hash)
}

/// Keep this content in the store (hard link for archives, copy for everything else).
pub(crate) fn store(game: &Path, src: &Path, hash: &str) -> Result<(), String> {
    let dest = object(game, hash);
    if dest.exists() {
        return Ok(());
    }
    fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let archive = src.extension().is_some_and(|e| e == "jar" || e == "zip");
    // Configs are often rewritten in place, which would change a hard-linked copy too; jars are
    // replaced, never edited, so linking them is safe and free.
    if archive && fs::hard_link(src, &dest).is_ok() {
        return Ok(());
    }
    let tmp = dest.with_extension("tmp");
    fs::copy(src, &tmp).map_err(|e| format!("{}: {e}", src.display()))?;
    fs::rename(&tmp, &dest).map_err(|e| e.to_string())
}

fn manifest_path(game: &Path, id: &str) -> PathBuf {
    root(game).join("manifests").join(format!("{id}.json"))
}

fn read_manifest(game: &Path, id: &str) -> Vec<FileEntry> {
    fs::read_to_string(manifest_path(game, id)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// All backups, newest first.
pub fn list(game: &Path) -> Vec<Backup> {
    let mut out: Vec<Backup> = fs::read_dir(root(game).join("backups")).into_iter().flatten().flatten()
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a: &Backup, b: &Backup| b.at_ms.cmp(&a.at_ms).then(b.id.cmp(&a.id)));
    out
}

/// The pack's files with hashes, reusing the last backup's hashes for files that haven't changed.
pub(crate) fn current(game: &Path, prev: &[FileEntry]) -> Result<Vec<FileEntry>, String> {
    let known: HashMap<&str, &FileEntry> = prev.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut out = Vec::new();
    for (path, (size, mtime)) in scan(game) {
        let hash = match known.get(path.as_str()) {
            Some(e) if e.size == size && e.mtime_ms == mtime => e.hash.clone(),
            _ => match real(game, &path).map(|p| hash_file(&p)) {
                Some(Ok(h)) => h,
                _ => continue, // vanished or unreadable mid-scan
            },
        };
        out.push(FileEntry { path, size, mtime_ms: mtime, hash });
    }
    Ok(out)
}

/// Mod id, name and version of a jar (from its own metadata).
fn mod_label(p: &Path) -> Option<(String, String, String)> {
    let f = fs::File::open(p).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let mut text = |n: &str| -> Option<String> {
        let mut s = String::new();
        z.by_name(n).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    for t in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(doc) = text(t).and_then(|s| s.parse::<toml::Table>().ok()) {
            let m = doc.get("mods")?.as_array()?.first()?;
            let s = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            return Some((s("modId"), s("displayName"), s("version")));
        }
    }
    let v: serde_json::Value = serde_json::from_str(&text("fabric.mod.json")?).ok()?;
    Some((v["id"].as_str()?.into(), v["name"].as_str().unwrap_or_default().into(), v["version"].as_str().unwrap_or_default().into()))
}

/// What changed between two file lists. `old_dir`/`new_dir` find each side's content (for mod names).
pub(crate) fn diff(game: &Path, old: &[FileEntry], new: &[FileEntry], new_is_live: bool) -> Vec<Change> {
    let o: HashMap<&str, &FileEntry> = old.iter().map(|e| (e.path.as_str(), e)).collect();
    let n: HashMap<&str, &FileEntry> = new.iter().map(|e| (e.path.as_str(), e)).collect();
    let file_of = |e: &FileEntry, live: bool| if live { real(game, &e.path) } else { Some(object(game, &e.hash)) };
    let jar = |p: &str| p.starts_with("mods/") && p.ends_with(".jar");
    let short = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();

    let mut added: Vec<&FileEntry> = new.iter().filter(|e| !o.contains_key(e.path.as_str())).collect();
    let mut removed: Vec<&FileEntry> = old.iter().filter(|e| !n.contains_key(e.path.as_str())).collect();
    let mut out = Vec::new();

    // A mod jar swapped for another file of the same mod is an update, not a remove + add.
    let label_of = |e: &FileEntry, live: bool| file_of(e, live).and_then(|p| mod_label(&p));
    let mut removed_ids: HashMap<String, (usize, String, String)> = HashMap::new();
    for (i, r) in removed.iter().enumerate() {
        if jar(&r.path) {
            if let Some((id, name, ver)) = label_of(r, false) {
                removed_ids.insert(id, (i, name, ver));
            }
        }
    }
    let mut used_removed: HashSet<usize> = HashSet::new();
    let mut used_added: HashSet<usize> = HashSet::new();
    for (k, a) in added.iter().enumerate() {
        if !jar(&a.path) {
            continue;
        }
        let Some((id, name, ver)) = label_of(a, new_is_live) else { continue };
        if let Some((i, _, old_ver)) = removed_ids.get(&id) {
            used_removed.insert(*i);
            used_added.insert(k);
            let name = if name.is_empty() { id.clone() } else { name };
            let label = if old_ver != &ver && !ver.is_empty() { format!("{name} {old_ver} → {ver}") } else { format!("{name} replaced") };
            out.push(Change { kind: "changed".into(), area: "mods".into(), path: a.path.clone(), label, details: vec![] });
        }
    }
    let mut keep = 0;
    removed.retain(|_| { let k = !used_removed.contains(&keep); keep += 1; k });
    keep = 0;
    added.retain(|_| { let k = !used_added.contains(&keep); keep += 1; k });

    let nice = |e: &FileEntry, live: bool| {
        if jar(&e.path) {
            if let Some((id, name, ver)) = label_of(e, live) {
                return format!("{} {ver}", if name.is_empty() { id } else { name }).trim().to_string();
            }
        }
        if e.path == PACK_FILE {
            return "Minecraft / loader version".into();
        }
        short(&e.path)
    };
    for a in added {
        out.push(Change { kind: "added".into(), area: area_of(&a.path).into(), path: a.path.clone(), label: nice(a, new_is_live), details: vec![] });
    }
    for r in removed {
        out.push(Change { kind: "removed".into(), area: area_of(&r.path).into(), path: r.path.clone(), label: nice(r, false), details: vec![] });
    }
    for e in new {
        if let Some(prev) = o.get(e.path.as_str()) {
            if prev.hash != e.hash {
                let label = if e.path == PACK_FILE { "Minecraft / loader version".to_string() } else { e.path.trim_start_matches("config/").to_string() };
                let text = |p: Option<PathBuf>| p.and_then(|p| fs::read_to_string(p).ok());
                let details = match (text(Some(object(game, &prev.hash))), text(file_of(e, new_is_live))) {
                    (Some(a), Some(b)) => field_changes(&e.path, &a, &b),
                    _ => vec![],
                };
                out.push(Change { kind: "changed".into(), area: area_of(&e.path).into(), path: e.path.clone(), label, details });
            }
        }
    }
    out.sort_by(|a, b| a.area.cmp(&b.area).then(a.kind.cmp(&b.kind)).then(a.label.to_lowercase().cmp(&b.label.to_lowercase())));
    out
}

/// A config file's settings as flat "key → value" pairs, by its format; None when it doesn't parse.
fn settings_of(path: &str, text: &str) -> Option<BTreeMap<String, String>> {
    fn flat_json(prefix: &str, v: &serde_json::Value, out: &mut BTreeMap<String, String>) {
        match v {
            serde_json::Value::Object(m) if !m.is_empty() => {
                for (k, v) in m { flat_json(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, v, out); }
            }
            _ => { out.insert(prefix.to_string(), v.to_string()); }
        }
    }
    fn flat_toml(prefix: &str, v: &toml::Value, out: &mut BTreeMap<String, String>) {
        match v {
            toml::Value::Table(m) if !m.is_empty() => {
                for (k, v) in m { flat_toml(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, v, out); }
            }
            _ => { out.insert(prefix.to_string(), v.to_string()); }
        }
    }
    let ext = path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    let mut out = BTreeMap::new();
    match ext.as_str() {
        "toml" => flat_toml("", &toml::Value::Table(text.parse::<toml::Table>().ok()?), &mut out),
        "json" | "json5" | "jsonc" | "mcmeta" => flat_json("", &serde_json::from_str(text).ok()?, &mut out),
        "properties" | "cfg" | "txt" | "ini" | "conf" => {
            // key=value or key:value lines, with [sections] (ini) and `name {` blocks (old Forge .cfg).
            let mut stack: Vec<String> = Vec::new();
            let mut section = String::new();
            for line in text.lines().map(str::trim) {
                if line.is_empty() || line.starts_with(['#', '!', ';']) || line.starts_with("//") { continue; }
                if line == "}" { stack.pop(); continue; }
                if let Some(name) = line.strip_suffix('{') { stack.push(name.trim().trim_matches('"').to_string()); continue; }
                if line.starts_with('[') && line.ends_with(']') { section = line[1..line.len() - 1].to_string(); continue; }
                // Forge .cfg type prefixes (S:, I:, B:) go before the key.
                let body = match line.as_bytes() { [t, b':', ..] if t.is_ascii_uppercase() => &line[2..], _ => line };
                let Some(i) = body.find(['=', ':']) else { continue };
                let mut full: Vec<&str> = stack.iter().map(String::as_str).collect();
                if !section.is_empty() { full.insert(0, &section); }
                full.push(body[..i].trim());
                out.insert(full.join("."), body[i + 1..].trim().to_string());
            }
        }
        _ => return None,
    }
    Some(out)
}

/// The settings that differ between two versions of a config file, at most 30 lines.
pub(crate) fn field_changes(path: &str, old: &str, new: &str) -> Vec<String> {
    let (Some(a), Some(b)) = (settings_of(path, old), settings_of(path, new)) else { return vec![] };
    let cut = |v: &str| if v.chars().count() > 60 { format!("{}…", v.chars().take(60).collect::<String>()) } else { v.to_string() };
    let mut out: Vec<String> = Vec::new();
    for (k, v) in &b {
        match a.get(k) {
            Some(old) if old != v => out.push(format!("{k}: {} → {}", cut(old), cut(v))),
            None => out.push(format!("{k}: added ({})", cut(v))),
            _ => {}
        }
    }
    out.extend(a.iter().filter(|(k, _)| !b.contains_key(*k)).map(|(k, v)| format!("{k}: removed (was {})", cut(v))));
    if out.len() > 30 {
        let more = out.len() - 30;
        out.truncate(30);
        out.push(format!("…and {more} more"));
    }
    out
}

/// Local "YYYY-MM-DD HH:MM" for a time in ms.
fn local_stamp(ms: u64) -> String {
    let t = ms as i64 + crate::timeline::local_offset_ms();
    let mins = t.div_euclid(60_000);
    // civil-from-days (Howard Hinnant)
    let z = t.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{}-{m:02}-{d:02} {:02}:{:02}", yoe + era * 400 + i64::from(m <= 2), mins.rem_euclid(1440) / 60, mins.rem_euclid(60))
}

/// The readable changelog: `.obelisk/changelog.md` in the game folder, newest entry last.
pub fn changelog_path(game: &Path) -> PathBuf {
    game.join(".obelisk").join("changelog.md")
}

/// Append a backup's changes to the changelog.
fn log_changes(game: &Path, b: &Backup) {
    if b.changes.is_empty() { return; }
    let what = if b.label.is_empty() { b.kind.as_str() } else { b.label.as_str() };
    let mut t = format!("\n## {} · {what} (backup {})\n", local_stamp(b.at_ms), b.id);
    for c in &b.changes {
        t.push_str(&format!("- {} {}: {}\n", c.kind, c.area, c.label));
        for d in &c.details { t.push_str(&format!("  - {d}\n")); }
    }
    let p = changelog_path(game);
    if !p.exists() { t.insert_str(0, "# Pack changes\n\nObelisk adds an entry each time it backs up a change to this pack.\n"); }
    use std::io::Write;
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) { let _ = f.write_all(t.as_bytes()); }
}

/// What changed since the newest backup (or everything, if there is none yet).
pub fn pending(game: &Path) -> Result<Vec<Change>, String> {
    let last = list(game).into_iter().next();
    let prev = last.as_ref().map(|b| read_manifest(game, &b.id)).unwrap_or_default();
    if last.is_none() {
        return Ok(vec![]);
    }
    let now = current(game, &prev)?;
    Ok(diff(game, &prev, &now, true))
}

/// Take a backup. With `only_if_changed`, nothing happens when the pack matches the newest backup.
pub fn create(game: &Path, kind: &str, label: &str, only_if_changed: bool) -> Result<Option<Backup>, String> {
    let last = list(game).into_iter().next();
    let prev = last.as_ref().map(|b| read_manifest(game, &b.id)).unwrap_or_default();
    let files = current(game, &prev)?;
    let changes = if last.is_some() { diff(game, &prev, &files, true) } else { vec![] };
    if only_if_changed && last.is_some() && changes.is_empty() {
        return Ok(None);
    }
    for f in &files {
        if let Some(p) = real(game, &f.path) {
            store(game, &p, &f.hash)?;
        }
    }
    let at = now_ms();
    let mut id = format!("{at}");
    let mut n = 1;
    while root(game).join("backups").join(format!("{id}.json")).exists() {
        id = format!("{at}-{n}");
        n += 1;
    }
    let backup = Backup {
        id: id.clone(), at_ms: at, kind: kind.into(), label: label.into(),
        files: files.len(), bytes: files.iter().map(|f| f.size).sum(), changes,
    };
    fs::create_dir_all(root(game).join("manifests")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root(game).join("backups")).map_err(|e| e.to_string())?;
    fs::write(manifest_path(game, &id), serde_json::to_vec(&files).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::write(root(game).join("backups").join(format!("{id}.json")), serde_json::to_vec_pretty(&backup).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    log_changes(game, &backup);
    Ok(Some(backup))
}

/// Before a launch and when Obelisk starts: back up the pack only if it changed since the newest backup.
pub fn on_change(game: &Path, label: &str) -> Result<Option<Backup>, String> {
    create(game, "auto", label, true)
}

/// Auto backup for the noon/midnight slot that started at `slot_ms` (worked out by the UI, which
/// knows the local time zone): taken only if the pack changed and this slot doesn't have one yet.
pub fn auto(game: &Path, slot_ms: u64) -> Result<Option<Backup>, String> {
    if list(game).iter().any(|b| b.kind == "auto" && b.at_ms >= slot_ms) {
        return Ok(None);
    }
    create(game, "auto", "", true)
}

fn backup_by_id(game: &Path, id: &str) -> Result<Backup, String> {
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return Err("invalid backup".into());
    }
    list(game).into_iter().find(|b| b.id == id).ok_or_else(|| "that backup no longer exists".into())
}

/// Put one file back to how it is in `entry` (or remove it when `entry` is None).
pub(crate) fn put(game: &Path, path: &str, entry: Option<&FileEntry>) -> Result<(), String> {
    let dest = real(game, path).ok_or("invalid path")?;
    match entry {
        Some(e) => {
            let src = object(game, &e.hash);
            if !src.is_file() {
                return Err(format!("{path}: its saved copy is missing"));
            }
            if dest.is_file() && hash_file(&dest).ok().as_deref() == Some(e.hash.as_str()) {
                return Ok(());
            }
            fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
            // Copy (not link) back, so later edits to the pack never touch the store.
            let tmp = dest.with_extension("obelisk-restore");
            fs::copy(&src, &tmp).map_err(|e| format!("{path}: {e}"))?;
            if dest.exists() {
                fs::remove_file(&dest).map_err(|e| format!("{path}: {e}"))?;
            }
            fs::rename(&tmp, &dest).map_err(|e| format!("{path}: {e}"))
        }
        None => match fs::remove_file(&dest) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{path}: {e}")),
        },
    }
}

/// Make the pack match a backup. A safety backup of the current state is taken first, so a
/// restore can itself be undone.
pub fn restore(game: &Path, id: &str) -> Result<Vec<Change>, String> {
    let target = backup_by_id(game, id)?;
    create(game, "safety", &format!("Before restoring {}", target.id), true)?;
    let want = read_manifest(game, id);
    let now = current(game, &read_manifest(game, &list(game)[0].id))?;
    let changes = diff(game, &now, &want, false);
    let want_map: HashMap<&str, &FileEntry> = want.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut errors = Vec::new();
    for e in &now {
        if !want_map.contains_key(e.path.as_str()) {
            if let Err(err) = put(game, &e.path, None) { errors.push(err); }
        }
    }
    for e in &want {
        if let Err(err) = put(game, &e.path, Some(e)) { errors.push(err); }
    }
    if errors.is_empty() { Ok(changes) } else { Err(errors.join("\n")) }
}

/// Undo one change from a backup's changelog: put that file back the way it was in the backup
/// before it (or remove it, if it didn't exist then).
pub fn revert_change(game: &Path, id: &str, path: &str) -> Result<(), String> {
    let all = list(game);
    let pos = all.iter().position(|b| b.id == id).ok_or("that backup no longer exists")?;
    let before = all.get(pos + 1).map(|b| read_manifest(game, &b.id)).unwrap_or_default();
    create(game, "safety", &format!("Before undoing a change to {path}"), true)?;
    // An updated mod lives under its new file name; the old file name is the one to bring back.
    let change = all[pos].changes.iter().find(|c| c.path == path).cloned();
    if let Some(c) = &change {
        if c.kind == "changed" && c.area == "mods" && !before.iter().any(|e| e.path == path) {
            let new_manifest = read_manifest(game, id);
            let gone: Vec<&FileEntry> = before.iter().filter(|e| e.path.starts_with("mods/") && !new_manifest.iter().any(|n| n.path == e.path)).collect();
            let new_id = real(game, path).and_then(|p| mod_label(&p)).map(|l| l.0);
            if let Some(old) = gone.into_iter().find(|e| mod_label(&object(game, &e.hash)).map(|l| l.0) == new_id) {
                put(game, path, None)?;
                return put(game, &old.path, Some(old));
            }
        }
    }
    put(game, path, before.iter().find(|e| e.path == path))
}

/// Keep the newest `keep_auto` auto backups and `keep_manual` checkpoints (safety backups count
/// as auto), then delete stored content no backup refers to.
pub fn prune(game: &Path, keep_auto: usize, keep_manual: usize) -> Result<usize, String> {
    let all = list(game);
    let (mut auto_n, mut manual_n, mut removed) = (0, 0, 0);
    let (offset, now, mut days) = (crate::timeline::local_offset_ms(), now_ms(), HashSet::new());
    for b in &all {
        // Automatic ones: the newest `keep_auto`, plus the newest of each day for the last 30 days.
        let day = (b.at_ms as i64 + offset).div_euclid(86_400_000);
        let first_of_day = b.kind != "manual" && b.at_ms + 30 * 86_400_000 >= now && days.insert(day);
        let keep = if b.kind == "manual" { manual_n += 1; manual_n <= keep_manual } else { auto_n += 1; auto_n <= keep_auto || first_of_day };
        if !keep {
            let _ = fs::remove_file(root(game).join("backups").join(format!("{}.json", b.id)));
            let _ = fs::remove_file(manifest_path(game, &b.id));
            removed += 1;
        }
    }
    if removed > 0 {
        gc(game);
    }
    Ok(removed)
}

/// Delete stored content that no remaining backup refers to.
fn gc(game: &Path) {
    let mut live: HashSet<String> = list(game).iter().flat_map(|b| read_manifest(game, &b.id)).map(|e| e.hash).collect();
    // A shared pack's log needs its files too (friends may still fetch them).
    live.extend(crate::share::referenced(game));
    for d in fs::read_dir(root(game).join("objects")).into_iter().flatten().flatten() {
        for f in fs::read_dir(d.path()).into_iter().flatten().flatten() {
            if !live.contains(&f.file_name().to_string_lossy().to_string()) {
                let _ = fs::remove_file(f.path());
            }
        }
    }
}

/// Space the backup store really uses on disk. Hard-linked jars that are still in the pack are
/// free, so they're not counted.
pub fn disk_use(game: &Path) -> u64 {
    let in_pack: HashSet<String> = list(game).first().map(|b| read_manifest(game, &b.id)).unwrap_or_default()
        .into_iter().filter(|e| real(game, &e.path).is_some_and(|p| p.is_file())).map(|e| e.hash).collect();
    let mut total = 0;
    for d in fs::read_dir(root(game).join("objects")).into_iter().flatten().flatten() {
        for f in fs::read_dir(d.path()).into_iter().flatten().flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            let linked = f.path().extension().is_none() && in_pack.contains(&name)
                && f.metadata().is_ok_and(|m| m.len() > 0) && is_hard_linked(&f.path());
            if !linked {
                total += f.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

#[cfg(windows)]
fn is_hard_linked(p: &Path) -> bool {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Info { attrs: u32, c: [u32; 2], a: [u32; 2], w: [u32; 2], serial: u32, size_hi: u32, size_lo: u32, links: u32, idx_hi: u32, idx_lo: u32 }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(h: *mut std::ffi::c_void, info: *mut Info) -> i32;
    }
    let Ok(f) = fs::File::open(p) else { return false };
    let mut info = std::mem::MaybeUninit::<Info>::zeroed();
    let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle(), info.as_mut_ptr()) } != 0;
    ok && unsafe { info.assume_init() }.links > 1
}

#[cfg(not(windows))]
fn is_hard_linked(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(p).is_ok_and(|m| m.nlink() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(p: &Path, id: &str, version: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(fs::File::create(p).unwrap());
        z.start_file("META-INF/neoforge.mods.toml", zip::write::SimpleFileOptions::default()).unwrap();
        write!(z, "[[mods]]\nmodId=\"{id}\"\ndisplayName=\"{id}\"\nversion=\"{version}\"\n").unwrap();
        z.finish().unwrap();
    }

    fn pack() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("minecraft");
        jar(&game.join("mods/create-1.0.jar"), "create", "1.0");
        jar(&game.join("mods/jei-2.0.jar"), "jei", "2.0");
        fs::create_dir_all(game.join("config")).unwrap();
        fs::write(game.join("config/create.toml"), "speed = 1").unwrap();
        fs::write(d.path().join("mmc-pack.json"), "{}").unwrap();
        fs::create_dir_all(game.join("saves/World")).unwrap();
        fs::write(game.join("saves/World/level.dat"), "big world").unwrap();
        (d, game)
    }

    #[test]
    fn backups_track_changes_and_restore() {
        let (_d, game) = pack();
        let first = create(&game, "manual", "Start", false).unwrap().unwrap();
        assert!(first.changes.is_empty());
        assert_eq!(first.files, 4); // 2 jars, 1 config, mmc-pack.json; worlds are left out
        assert!(create(&game, "auto", "", true).unwrap().is_none(), "no changes, no backup");

        // Update a mod, delete one, edit a config.
        fs::remove_file(game.join("mods/create-1.0.jar")).unwrap();
        jar(&game.join("mods/create-1.1.jar"), "create", "1.1");
        fs::remove_file(game.join("mods/jei-2.0.jar")).unwrap();
        fs::write(game.join("config/create.toml"), "speed = 9").unwrap();
        let labels: Vec<String> = pending(&game).unwrap().iter().map(|c| format!("{} {}", c.kind, c.label)).collect();
        assert!(labels.contains(&"changed create 1.0 → 1.1".to_string()), "{labels:?}");
        assert!(labels.contains(&"removed jei 2.0".to_string()), "{labels:?}");
        assert!(labels.contains(&"changed create.toml".to_string()), "{labels:?}");
        let second = create(&game, "auto", "", true).unwrap().unwrap();
        assert_eq!(second.changes.len(), 3);
        let cfg = second.changes.iter().find(|c| c.path == "config/create.toml").unwrap();
        assert_eq!(cfg.details, vec!["speed: 1 → 9".to_string()]);
        assert!(fs::read_to_string(changelog_path(&game)).unwrap().contains("  - speed: 1 → 9"));

        // Undo just the JEI removal.
        revert_change(&game, &second.id, "mods/jei-2.0.jar").unwrap();
        assert!(game.join("mods/jei-2.0.jar").is_file());
        // Undo just the Create update: back to 1.0.
        revert_change(&game, &second.id, "mods/create-1.1.jar").unwrap();
        assert!(game.join("mods/create-1.0.jar").is_file() && !game.join("mods/create-1.1.jar").exists());

        // Restore everything to the first backup.
        fs::write(game.join("config/create.toml"), "speed = 42").unwrap();
        restore(&game, &first.id).unwrap();
        assert_eq!(fs::read_to_string(game.join("config/create.toml")).unwrap(), "speed = 1");
        assert!(game.join("mods/create-1.0.jar").is_file() && game.join("mods/jei-2.0.jar").is_file());
        assert!(game.join("saves/World/level.dat").is_file(), "worlds are never touched");
        assert!(list(&game).iter().any(|b| b.kind == "safety"), "restores can be undone");
    }

    #[test]
    #[ignore] // a real pack: OBELISK_TEST_GAME=<copy of a game folder> cargo test real_pack -- --ignored --nocapture
    fn real_pack() {
        let game = PathBuf::from(std::env::var("OBELISK_TEST_GAME").unwrap());
        let t = std::time::Instant::now();
        let first = on_change(&game, "First").unwrap();
        println!("first backup: {:?} in {:?}", first.map(|b| b.files), t.elapsed());
        let t = std::time::Instant::now();
        assert!(on_change(&game, "Again").unwrap().is_none(), "no change, no backup");
        println!("unchanged check: {:?}", t.elapsed());
        let cfg = fs::read_dir(game.join("config")).unwrap().flatten().map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "toml") && fs::read_to_string(p).is_ok_and(|s| s.contains("= true"))).unwrap();
        let text = fs::read_to_string(&cfg).unwrap();
        fs::write(&cfg, text.replacen("= true", "= false", 1)).unwrap();
        let b = on_change(&game, "Before launch").unwrap().unwrap();
        println!("{:#?}", b.changes);
        println!("{}", fs::read_to_string(changelog_path(&game)).unwrap());
    }

    #[test]
    fn config_field_changes() {
        let c = field_changes("config/x.json", r#"{"a":{"b":1,"c":"x"}}"#, r#"{"a":{"b":2},"d":true}"#);
        assert_eq!(c, vec!["a.b: 1 → 2", "d: added (true)", "a.c: removed (was \"x\")"]);
        let old = "# c
general {
    I:distance=8
    S:name=a
}
";
        let new = "# c
general {
    I:distance=12
    S:name=a
}
";
        assert_eq!(field_changes("config/old.cfg", old, new), vec!["general.distance: 8 → 12"]);
        assert_eq!(field_changes("options.txt", "fov:0.5
gamma:1", "fov:0.7
gamma:1"), vec!["fov: 0.5 → 0.7"]);
        assert!(field_changes("config/a.png", "x", "y").is_empty());
    }

    #[test]
    fn auto_once_per_slot_and_pruning() {
        let (_d, game) = pack();
        create(&game, "manual", "Start", false).unwrap();
        fs::write(game.join("config/create.toml"), "a").unwrap();
        let slot = now_ms() - 1000;
        assert!(auto(&game, slot).unwrap().is_some());
        fs::write(game.join("config/create.toml"), "b").unwrap();
        assert!(auto(&game, slot).unwrap().is_none(), "one auto backup per noon/midnight slot");
        for i in 0..5 {
            fs::write(game.join("config/create.toml"), format!("v{i}")).unwrap();
            create(&game, "auto", "", true).unwrap();
        }
        prune(&game, 2, 10).unwrap();
        let left = list(&game);
        assert_eq!(left.iter().filter(|b| b.kind == "auto").count(), 2);
        assert_eq!(left.iter().filter(|b| b.kind == "manual").count(), 1, "checkpoints have their own limit");
        // Content only the pruned backups used is gone; what's still referenced remains restorable.
        let manual = left.iter().find(|b| b.kind == "manual").unwrap().id.clone();
        restore(&game, &manual).unwrap();
        assert_eq!(fs::read_to_string(game.join("config/create.toml")).unwrap(), "speed = 1");
    }
}
