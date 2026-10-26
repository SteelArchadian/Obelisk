//! Modpack trash: a deleted profile is moved (not copied) into `<instances>/.obelisk-trash/` and kept
//! for a while so it can be put back. Prism ignores the folder, since it has no instance.cfg at its top.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrashedPack {
    /// Folder name inside the trash.
    pub id: String,
    /// The profile's folder name before it was deleted.
    pub instance_id: String,
    pub name: String,
    pub deleted_ms: u64,
    pub bytes: u64,
}

fn root(instances: &Path) -> PathBuf {
    instances.join(".obelisk-trash")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn size(p: &Path) -> u64 {
    fs::read_dir(p).into_iter().flatten().flatten().map(|e| {
        let m = e.metadata();
        if m.as_ref().is_ok_and(|m| m.is_dir()) { size(&e.path()) } else { m.map(|m| m.len()).unwrap_or(0) }
    }).sum()
}

fn checked(instances: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        return Err("invalid trash entry".into());
    }
    let dir = root(instances).join(id);
    if !dir.join("obelisk-trash.json").is_file() {
        return Err("that modpack is no longer in the trash".into());
    }
    Ok(dir)
}

/// Move a profile into the trash.
pub fn put(instances: &Path, instance_id: &str, name: &str) -> Result<TrashedPack, String> {
    if instance_id.is_empty() || instance_id.contains(['/', '\\']) || instance_id.contains("..") {
        return Err("invalid profile".into());
    }
    let src = instances.join(instance_id);
    if !src.join("instance.cfg").is_file() {
        return Err(format!("profile {instance_id} not found"));
    }
    let at = now_ms();
    let id = format!("{at}-{}", instance_id.chars().filter(|c| c.is_alphanumeric() || "-_ ".contains(*c)).collect::<String>());
    let dir = root(instances).join(&id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let entry = TrashedPack { id: id.clone(), instance_id: instance_id.into(), name: name.into(), deleted_ms: at, bytes: size(&src) };
    // A rename within the instances folder: instant, whatever the pack's size.
    fs::rename(&src, dir.join("pack")).map_err(|e| {
        let _ = fs::remove_dir_all(&dir);
        format!("Couldn't move the profile (is Minecraft or Prism using it?): {e}")
    })?;
    fs::write(dir.join("obelisk-trash.json"), serde_json::to_vec_pretty(&entry).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(entry)
}

/// Trashed modpacks, newest first.
pub fn list(instances: &Path) -> Vec<TrashedPack> {
    let mut v: Vec<TrashedPack> = fs::read_dir(root(instances)).into_iter().flatten().flatten()
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path().join("obelisk-trash.json")).ok()?).ok())
        .collect();
    v.sort_by(|a, b| b.deleted_ms.cmp(&a.deleted_ms));
    v
}

/// Put a modpack back. If a profile with its folder name exists again, it comes back as "<name> (restored)".
pub fn restore(instances: &Path, id: &str) -> Result<String, String> {
    let dir = checked(instances, id)?;
    let entry: TrashedPack = serde_json::from_str(&fs::read_to_string(dir.join("obelisk-trash.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let target = (1..).map(|i| match i { 1 => entry.instance_id.clone(), 2 => format!("{} (restored)", entry.instance_id), n => format!("{} (restored {n})", entry.instance_id) })
        .find(|n| !instances.join(n).exists()).unwrap();
    fs::rename(dir.join("pack"), instances.join(&target)).map_err(|e| e.to_string())?;
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Delete one trashed modpack for good (via `remove`, e.g. the Recycle Bin).
pub fn remove(instances: &Path, id: &str, remove: impl Fn(&Path) -> Result<(), String>) -> Result<(), String> {
    remove(&checked(instances, id)?)
}

/// Delete modpacks that have been in the trash longer than `max_ms`.
pub fn purge(instances: &Path, max_ms: u64, remove: impl Fn(&Path) -> Result<(), String>) -> usize {
    let cutoff = now_ms().saturating_sub(max_ms);
    let mut n = 0;
    for p in list(instances) {
        if p.deleted_ms < cutoff && remove(&root(instances).join(&p.id)).is_ok() {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_and_restore() {
        let d = tempfile::tempdir().unwrap();
        let inst = d.path().join("Pack");
        fs::create_dir_all(inst.join("minecraft/mods")).unwrap();
        fs::write(inst.join("instance.cfg"), "name=Pack").unwrap();
        fs::write(inst.join("minecraft/mods/a.jar"), "x").unwrap();
        let t = put(d.path(), "Pack", "Pack").unwrap();
        assert!(!inst.exists());
        assert_eq!(list(d.path()).len(), 1);
        // Same name taken meanwhile: comes back alongside it.
        fs::create_dir_all(&inst).unwrap();
        assert_eq!(restore(d.path(), &t.id).unwrap(), "Pack (restored)");
        assert!(d.path().join("Pack (restored)/minecraft/mods/a.jar").is_file());
        assert!(list(d.path()).is_empty());
        assert_eq!(purge(d.path(), 0, |p| fs::remove_dir_all(p).map_err(|e| e.to_string())), 0);
    }
}
