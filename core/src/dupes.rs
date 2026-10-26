//! The same mod installed twice, usually once from Modrinth and once from CurseForge (their files
//! differ, so ids and hashes don't match, but the mod id inside the jar does). Two enabled jars with
//! one mod id stop NeoForge/Forge/Fabric from starting, so installs refuse a second copy and the
//! mods list flags any that slipped in.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// The mod ids a jar itself declares (not the libraries bundled inside it, which loaders dedupe).
pub fn own_ids(jar: &Path) -> Vec<String> {
    fs::File::open(jar).map(|f| crate::configs::ids_in(f, 1)).unwrap_or_default()
}

pub fn own_ids_of(bytes: &[u8]) -> Vec<String> {
    crate::configs::ids_in(std::io::Cursor::new(bytes), 1)
}

/// Mod id -> file names, for the enabled jars in a mods folder.
pub fn index(mods: &Path) -> HashMap<String, Vec<String>> {
    let mut by_id: HashMap<String, Vec<String>> = HashMap::new();
    for e in fs::read_dir(mods).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().ends_with(".jar") { continue }
        for id in own_ids(&e.path()) {
            let files = by_id.entry(id).or_default();
            if !files.contains(&name) { files.push(name.clone()) }
        }
    }
    by_id
}

/// An installed jar (other than `file_name` itself) that already provides one of `ids`.
pub fn clash(index: &HashMap<String, Vec<String>>, ids: &[String], file_name: &str) -> Option<(String, String)> {
    ids.iter().find_map(|id| {
        index.get(id)?.iter().find(|f| f.as_str() != file_name).map(|f| (id.clone(), f.clone()))
    })
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Duplicate {
    pub mod_id: String,
    pub files: Vec<String>,
}

/// Mods present more than once among a profile's enabled jars.
pub fn find(game: &Path) -> Vec<Duplicate> {
    let mut out: Vec<Duplicate> = index(&game.join("mods")).into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(mod_id, mut files)| { files.sort(); Duplicate { mod_id, files } })
        .collect();
    out.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    out
}

/// Refuse to start a game with a mod installed twice: NeoForge stops at the loading screen anyway.
pub fn check_before_launch(game: &Path) -> Result<(), String> {
    let found = find(game);
    if found.is_empty() { return Ok(()) }
    let list: Vec<String> = found.iter().map(|d| format!("{} ({})", d.mod_id, d.files.join(", "))).collect();
    Err(format!("Not starting: {} installed twice: {}. Disable one copy of each in the mods list.",
        if found.len() == 1 { "a mod is".to_string() } else { format!("{} mods are", found.len()) }, list.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(dir: &Path, name: &str, id: &str) {
        let mut z = zip::ZipWriter::new(fs::File::create(dir.join(name)).unwrap());
        z.start_file("META-INF/neoforge.mods.toml", zip::write::SimpleFileOptions::default()).unwrap();
        write!(z, "[[mods]]\nmodId=\"{id}\"\n").unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn finds_the_same_mod_from_both_sites() {
        let game = std::env::temp_dir().join(format!("obelisk-dupes-{}", std::process::id()));
        let mods = game.join("mods");
        fs::create_dir_all(&mods).unwrap();
        jar(&mods, "chat_heads-0.15.7-neoforge-1.21.jar", "chat_heads");
        jar(&mods, "chat-heads-0.15.7-neoforge.jar", "chat_heads");
        jar(&mods, "jei-19.jar", "jei");
        jar(&mods, "old.jar.disabled", "jei");
        let d = find(&game);
        assert_eq!(d, vec![Duplicate { mod_id: "chat_heads".into(),
            files: vec!["chat-heads-0.15.7-neoforge.jar".into(), "chat_heads-0.15.7-neoforge-1.21.jar".into()] }]);
        let idx = index(&mods);
        assert_eq!(clash(&idx, &["jei".into()], "jei-20.jar"), Some(("jei".into(), "jei-19.jar".into())));
        assert_eq!(clash(&idx, &["jei".into()], "jei-19.jar"), None);
        let _ = fs::remove_dir_all(&game);
    }
}
