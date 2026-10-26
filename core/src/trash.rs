//! Per-profile trash: deleted content waits in `.obelisk/trash/` for a while so it can be put back.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    pub id: String,
    pub kind: String,
    pub file_name: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub authors: String,
    #[serde(default)]
    pub source: String,
    pub deleted_ms: u64,
    /// File name of the Prism `.index` entry that went with it, if any.
    pub index: Option<String>,
}

fn root(game: &Path) -> PathBuf {
    game.join(".obelisk").join("trash")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn entry_dir(game: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        return Err("invalid trash entry".into());
    }
    let dir = root(game).join(id);
    if !dir.join("entry.json").is_file() {
        return Err("that item is no longer in the trash".into());
    }
    Ok(dir)
}

/// Move a file (and its `.index` entry) from a content folder into the trash.
pub fn put(game: &Path, kind: &str, file_name: &str, name: &str, version: &str, authors: &str, source: &str) -> Result<TrashEntry, String> {
    crate::check_name(kind, file_name)?;
    let folder = game.join(kind);
    let path = folder.join(file_name);
    if !path.is_file() {
        return Err(format!("{file_name} not found"));
    }
    let deleted_ms = now_ms();
    let id = format!("{deleted_ms}-{}", file_name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-', "_"));
    let dir = root(game).join(&id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::rename(&path, dir.join(file_name)).map_err(|e| e.to_string())?;
    let index = crate::index_files_for(&folder, file_name).into_iter().next();
    let index_name = index.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
    if let (Some(p), Some(n)) = (&index, &index_name) {
        fs::rename(p, dir.join(n)).map_err(|e| e.to_string())?;
    }
    let entry = TrashEntry {
        id, kind: kind.into(), file_name: file_name.into(), name: name.into(),
        version: version.into(), authors: authors.into(), source: source.into(), deleted_ms, index: index_name,
    };
    fs::write(dir.join("entry.json"), serde_json::to_vec_pretty(&entry).unwrap()).map_err(|e| e.to_string())?;
    Ok(entry)
}

/// Everything in this profile's trash, newest first.
pub fn list(game: &Path) -> Vec<TrashEntry> {
    let Ok(entries) = fs::read_dir(root(game)) else { return vec![] };
    let mut out: Vec<TrashEntry> = entries.flatten()
        .filter_map(|e| serde_json::from_slice(&fs::read(e.path().join("entry.json")).ok()?).ok())
        .collect();
    out.sort_by(|a, b| b.deleted_ms.cmp(&a.deleted_ms));
    out
}

/// Put an item back where it was.
pub fn restore(game: &Path, id: &str) -> Result<TrashEntry, String> {
    let dir = entry_dir(game, id)?;
    let entry: TrashEntry = serde_json::from_slice(&fs::read(dir.join("entry.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    crate::check_name(&entry.kind, &entry.file_name)?;
    let folder = game.join(&entry.kind);
    let base = entry.file_name.trim_end_matches(".disabled");
    if folder.join(base).exists() || folder.join(format!("{base}.disabled")).exists() {
        return Err(format!("{} is already in this profile", entry.file_name));
    }
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    fs::rename(dir.join(&entry.file_name), folder.join(&entry.file_name)).map_err(|e| e.to_string())?;
    if let Some(n) = entry.index.as_ref().filter(|n| !n.contains(['/', '\\']) && !n.contains("..")) {
        fs::create_dir_all(folder.join(".index")).map_err(|e| e.to_string())?;
        let _ = fs::rename(dir.join(n), folder.join(".index").join(n));
    }
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(entry)
}

/// Delete one item for good.
pub fn remove(game: &Path, id: &str) -> Result<(), String> {
    fs::remove_dir_all(entry_dir(game, id)?).map_err(|e| e.to_string())
}

/// Delete everything older than `max_age_ms`. Returns how many items went.
pub fn purge(game: &Path, max_age_ms: u64) -> usize {
    let cutoff = now_ms().saturating_sub(max_age_ms);
    list(game).into_iter().filter(|e| e.deleted_ms < cutoff)
        .filter(|e| remove(game, &e.id).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_restore_and_expire() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        fs::create_dir_all(g.join("mods/.index")).unwrap();
        fs::write(g.join("mods/a.jar"), "jar").unwrap();
        fs::write(g.join("mods/.index/a.pw.toml"), "filename = 'a.jar'\n").unwrap();

        let e = put(g, "mods", "a.jar", "A", "1.0", "Me", "local").unwrap();
        assert!(!g.join("mods/a.jar").exists() && !g.join("mods/.index/a.pw.toml").exists());
        assert_eq!(list(g), vec![e.clone()]);

        restore(g, &e.id).unwrap();
        assert!(g.join("mods/a.jar").exists() && g.join("mods/.index/a.pw.toml").exists());
        assert!(list(g).is_empty());

        let e = put(g, "mods", "a.jar", "A", "1.0", "Me", "local").unwrap();
        assert_eq!(purge(g, 60_000), 0);
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(purge(g, 1), 1);
        assert!(restore(g, &e.id).is_err());
        assert!(restore(g, "../x").is_err());
    }
}
