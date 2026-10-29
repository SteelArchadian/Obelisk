//! Files from someone else's PC that Obelisk blocked because they act like account- or password-stealing code
//! (guard.rs), kept aside in the pack's `.obelisk/held` folder instead of being thrown away. The window lists them
//! with the reason; only the user can let one in (the app asks them in a Windows box, see share_allow_held), and a
//! newer version from a friend replaces the blocked one.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    /// Where it would go in the pack, like "mods/x.jar".
    pub path: String,
    pub hash: String,
    pub size: u64,
    /// Who sent it (a friend's name), empty when it came with a pack code.
    pub from: String,
    /// Why Obelisk blocked it (guard.rs's words, like "reads a launcher's saved Microsoft accounts").
    pub reasons: Vec<String>,
    pub at_ms: i64,
    /// The user chose to keep it blocked: still listed on the pack's page, but not asked about again.
    #[serde(default)]
    pub kept: bool,
}

fn dir(game: &Path) -> PathBuf {
    game.join(".obelisk").join("held")
}

fn index(game: &Path) -> PathBuf {
    dir(game).join("held.json")
}

fn save(game: &Path, list: &[Held]) -> Result<(), String> {
    fs::create_dir_all(dir(game)).map_err(|e| e.to_string())?;
    let tmp = index(game).with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, index(game)).map_err(|e| e.to_string())
}

/// The pack's blocked files.
pub fn list(game: &Path) -> Vec<Held> {
    fs::read(index(game)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Keep a blocked file aside (replacing an older one at the same place).
pub fn put(game: &Path, path: &str, bytes: &[u8], from: &str, reasons: Vec<String>) -> Result<(), String> {
    crate::history::real(game, path).ok_or("invalid path")?;
    let hash = crate::sha1_hex(bytes);
    fs::create_dir_all(dir(game)).map_err(|e| e.to_string())?;
    fs::write(dir(game).join(&hash), bytes).map_err(|e| e.to_string())?;
    let mut l = list(game);
    let old: Vec<String> = l.iter().filter(|h| h.path == path && h.hash != hash).map(|h| h.hash.clone()).collect();
    l.retain(|h| h.path != path);
    l.push(Held { path: path.into(), hash, size: bytes.len() as u64, from: from.into(), reasons, at_ms: crate::timeline::now_ms(), kept: false });
    save(game, &l)?;
    for h in old.into_iter().filter(|o| !l.iter().any(|x| &x.hash == o)) {
        let _ = fs::remove_file(dir(game).join(h));
    }
    Ok(())
}

/// Stop holding a file (a newer change replaced it, or it was let in).
pub fn forget(game: &Path, path: &str) -> Result<(), String> {
    let mut l = list(game);
    let Some(h) = l.iter().find(|h| h.path == path).cloned() else { return Ok(()) };
    l.retain(|x| x.path != path);
    save(game, &l)?;
    if !l.iter().any(|x| x.hash == h.hash) {
        let _ = fs::remove_file(dir(game).join(&h.hash));
    }
    Ok(())
}

/// The user chose "Keep it blocked": don't ask again.
pub fn keep(game: &Path, path: &str) -> Result<(), String> {
    let mut l = list(game);
    l.iter_mut().filter(|h| h.path == path).for_each(|h| h.kept = true);
    save(game, &l)
}

/// The user lets a blocked file in (after Windows asked them). Takes a safety backup first.
pub fn allow(game: &Path, path: &str) -> Result<(), String> {
    let h = list(game).into_iter().find(|h| h.path == path).ok_or("That file isn't blocked any more.")?;
    let bytes = fs::read(dir(game).join(&h.hash)).map_err(|_| "The blocked file is missing. Ask your friend to send it again.".to_string())?;
    if crate::sha1_hex(&bytes) != h.hash {
        return Err("The blocked file was changed on disk; not letting it in.".into());
    }
    let dest = crate::history::real(game, &h.path).ok_or("invalid path")?;
    crate::history::create(game, "safety", &format!("Before allowing {}", h.path), true)?;
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    forget(game, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_allow_and_replace() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path().join("minecraft");
        fs::create_dir_all(g.join("mods")).unwrap();
        put(&g, "mods/a.jar", b"v1", "Alex", vec!["reads things".into()]).unwrap();
        put(&g, "mods/a.jar", b"v2", "Alex", vec!["reads things".into()]).unwrap();
        assert_eq!(list(&g).len(), 1);
        assert!(!dir(&g).join(crate::sha1_hex(b"v1")).exists(), "the older one is gone");
        assert!(put(&g, "../x.jar", b"x", "", vec![]).is_err());
        keep(&g, "mods/a.jar").unwrap();
        assert!(list(&g)[0].kept);
        allow(&g, "mods/a.jar").unwrap();
        assert_eq!(fs::read(g.join("mods/a.jar")).unwrap(), b"v2");
        assert!(list(&g).is_empty() && !dir(&g).join(crate::sha1_hex(b"v2")).exists());
    }
}
