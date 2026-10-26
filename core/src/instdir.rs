//! Moving Prism's instances folder (where every modpack lives) somewhere else, e.g. next to Obelisk's
//! own data like CurseForge's `Instances` folder. Prism reads the folder from `InstanceDir` in
//! prismlauncher.cfg, so the move is: copy every pack over, check the copy, switch `InstanceDir`,
//! then remove the originals. Until the switch nothing has changed, so a failed copy is just cleaned up.

use std::fs;
use std::path::{Path, PathBuf};

/// Files and bytes under a folder (links are refused before this is used, so nothing is counted twice).
fn tally(p: &Path) -> std::io::Result<(u64, u64)> {
    let m = fs::symlink_metadata(p)?;
    if !m.is_dir() { return Ok((1, m.len())) }
    let mut t = (0, 0);
    for e in fs::read_dir(p)? {
        let (f, b) = tally(&e?.path())?;
        t.0 += f;
        t.1 += b;
    }
    Ok(t)
}

/// Links inside the packs (a symlink or junction to a mod's dev folder, say). Copying would follow them
/// and duplicate whatever they point to, so the move stops and names them instead.
fn links(p: &Path, out: &mut Vec<PathBuf>) {
    let Ok(m) = fs::symlink_metadata(p) else { return };
    if is_link(&m) { out.push(p.to_path_buf()); return }
    if m.is_dir() {
        for e in fs::read_dir(p).into_iter().flatten().flatten() { links(&e.path(), out) }
    }
}

#[cfg(windows)]
fn is_link(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT (junctions too)
}
#[cfg(not(windows))]
fn is_link(m: &fs::Metadata) -> bool { m.file_type().is_symlink() }

fn copy_all(src: &Path, dst: &Path, done: &mut u64, progress: &mut impl FnMut(u64)) -> std::io::Result<()> {
    if fs::symlink_metadata(src)?.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_all(&e.path(), &dst.join(e.file_name()), done, progress)?;
        }
        Ok(())
    } else {
        *done += fs::copy(src, dst)?;
        progress(*done);
        Ok(())
    }
}

fn remove(p: &Path) -> std::io::Result<()> {
    if fs::symlink_metadata(p)?.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

fn same(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    n(a) == n(b)
}

/// What a move would involve, checked before anything is touched.
pub struct Plan {
    pub from: PathBuf,
    pub to: PathBuf,
    pub names: Vec<std::ffi::OsString>,
    pub files: u64,
    pub bytes: u64,
}

/// Check a move of `data`'s instances folder to `to`: an absolute folder outside the current one,
/// with nothing of the same name there yet and no links inside the packs.
pub fn plan(data: &Path, to: &Path) -> Result<Plan, String> {
    let from = crate::instances_dir(data);
    if !to.is_absolute() { return Err("Choose a full folder path.".into()) }
    if same(&from, to) { return Err(format!("Your modpacks are already in {}.", to.display())) }
    if to.starts_with(&from) || from.starts_with(to) {
        return Err("The new folder can't be inside the current one (or the other way round).".into());
    }
    let names: Vec<_> = fs::read_dir(&from).map_err(|e| format!("{}: {e}", from.display()))?
        .flatten().map(|e| e.file_name()).collect();
    let clash: Vec<String> = names.iter().filter(|n| to.join(n).exists()).map(|n| n.to_string_lossy().into_owned()).collect();
    if !clash.is_empty() {
        return Err(format!("{} already has {}. Choose an empty folder.", to.display(), clash.join(", ")));
    }
    let mut found = Vec::new();
    for n in &names { links(&from.join(n), &mut found) }
    if !found.is_empty() {
        let list: Vec<String> = found.iter().take(5).map(|p| p.display().to_string()).collect();
        return Err(format!("These are links to other folders, which a move would copy in full: {}. Remove or replace them first.", list.join("; ")));
    }
    let (mut files, mut bytes) = (0, 0);
    for n in &names {
        let (f, b) = tally(&from.join(n)).map_err(|e| e.to_string())?;
        files += f;
        bytes += b;
    }
    Ok(Plan { from, to: to.to_path_buf(), names, files, bytes })
}

/// Carry out a checked plan. `progress(bytes copied)` as it goes. Prism and every game must be closed.
/// Returns anything that couldn't be removed from the old folder afterwards (the move itself succeeded).
pub fn run(data: &Path, plan: &Plan, mut progress: impl FnMut(u64)) -> Result<Vec<String>, String> {
    fs::create_dir_all(&plan.to).map_err(|e| format!("{}: {e}", plan.to.display()))?;
    let same_drive = plan.from.components().next() == plan.to.components().next();
    let mut placed: Vec<PathBuf> = Vec::new();
    let undo = |placed: &[PathBuf]| for p in placed { let _ = remove(p); };
    if same_drive {
        // One volume: renaming is instant and can't leave half a copy. Undo by renaming back.
        for n in &plan.names {
            if let Err(e) = fs::rename(plan.from.join(n), plan.to.join(n)) {
                for m in &plan.names {
                    if plan.to.join(m).exists() { let _ = fs::rename(plan.to.join(m), plan.from.join(m)); }
                }
                return Err(format!("couldn't move {}: {e}. Nothing was changed.", n.to_string_lossy()));
            }
        }
        progress(plan.bytes);
    } else {
        let mut done = 0;
        for n in &plan.names {
            let dst = plan.to.join(n);
            placed.push(dst.clone());
            if let Err(e) = copy_all(&plan.from.join(n), &dst, &mut done, &mut progress) {
                undo(&placed);
                return Err(format!("copying {} failed: {e}. Nothing was changed.", n.to_string_lossy()));
            }
        }
        let (mut files, mut bytes) = (0, 0);
        for n in &plan.names {
            let (f, b) = tally(&plan.to.join(n)).map_err(|e| e.to_string())?;
            files += f;
            bytes += b;
        }
        if (files, bytes) != (plan.files, plan.bytes) {
            undo(&placed);
            return Err(format!("the copy doesn't match ({files} files, {bytes} bytes; expected {}, {}). Nothing was changed.", plan.files, plan.bytes));
        }
    }
    if let Err(e) = crate::update_cfg(&data.join("prismlauncher.cfg"), &[("InstanceDir", Some(plan.to.display().to_string()))]) {
        if same_drive {
            for n in &plan.names { let _ = fs::rename(plan.to.join(n), plan.from.join(n)); }
        } else {
            undo(&placed);
        }
        return Err(format!("couldn't update Prism's settings: {e}. Nothing was changed."));
    }
    let mut left = Vec::new();
    if !same_drive {
        for n in &plan.names {
            if let Err(e) = remove(&plan.from.join(n)) { left.push(format!("{}: {e}", plan.from.join(n).display())) }
        }
    }
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_packs_and_points_prism_at_them() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("prism");
        let pack = data.join("instances/Pack/minecraft/mods");
        fs::create_dir_all(&pack).unwrap();
        fs::write(pack.join("a.jar"), "jar").unwrap();
        fs::write(data.join("instances/Pack/instance.cfg"), "name=Pack\n").unwrap();
        fs::write(data.join("prismlauncher.cfg"), "[General]\nLanguage=en\n").unwrap();
        let to = t.path().join("Obelisk").join("Instances");
        let p = plan(&data, &to).unwrap();
        assert_eq!((p.files, p.bytes), (2, 13));
        assert!(run(&data, &p, |_| {}).unwrap().is_empty());
        assert_eq!(crate::instances_dir(&data), to);
        assert!(to.join("Pack/minecraft/mods/a.jar").is_file() && !data.join("instances/Pack").exists());
        assert_eq!(crate::list_instances(&data).len(), 1);
        assert!(plan(&data, &to).is_err(), "already there");
    }
}
